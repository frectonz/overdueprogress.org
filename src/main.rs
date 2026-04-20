mod auth;
mod config;
mod error;
mod resend;
mod submissions;
mod telegram;
mod telemetry;
mod turnstile;
mod view;

#[cfg(test)]
mod tests;

use std::net::SocketAddr;
use std::time::Duration;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use minijinja::Value;
use rust_embed::EmbeddedFile;
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use tower_http::compression::CompressionLayer;
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;

use crate::auth::Auth;
use crate::config::Env;
use crate::view::{StaticAssets, View};

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub view: View,
    pub turnstile: turnstile::Client,
    pub resend: resend::Client,
    pub telegram: telegram::Client,
    pub auth: Auth,
}

impl AppState {
    pub fn notify_telegram(&self, template: &str, ctx: Value) {
        match self.view.render_to_string(template, ctx) {
            Ok(text) => self.telegram.notify(text),
            Err(err) => tracing::error!(?err, template, "telegram template render failed"),
        }
    }
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let env = Env::load();
    let _otel = telemetry::init(env.axiom_token.as_deref(), env.axiom_dataset.as_deref())?;
    tracing::info!(database_url = %env.database_url, "connecting to database");
    let db = connect_db(&env.database_url).await?;
    tracing::info!("running migrations");
    sqlx::migrate!("./migrations").run(&db).await?;
    tracing::info!("migrations applied");

    let http = reqwest::Client::new();
    tracing::info!(
        rp_id = %env.rp_id,
        rp_origin = %env.rp_origin,
        "initializing webauthn"
    );
    let state = AppState {
        db,
        view: View::new(),
        turnstile: turnstile::Client::new(
            http.clone(),
            env.turnstile_site_key,
            env.turnstile_secret_key,
        ),
        resend: resend::Client::new(http.clone(), env.resend_api_key, env.from_email),
        telegram: telegram::Client::new(http.clone(), env.telegram_bot_token, env.telegram_chat_id),
        auth: Auth::new(&env.rp_id, &env.rp_origin)?,
    };

    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(env.addr).await?;
    tracing::info!("listening on {}", listener.local_addr()?);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

async fn connect_db(url: &str) -> Result<SqlitePool, sqlx::Error> {
    let path = url.trim_start_matches("sqlite://");
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5))
        .pragma("cache_size", "-20000")
        .pragma("temp_store", "MEMORY");
    SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await
}

fn build_router(state: AppState) -> Router {
    Router::new()
        .merge(submissions::routes())
        .merge(auth::routes())
        .fallback(static_fallback)
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(CompressionLayer::new().br(true).gzip(true))
        .with_state(state)
}

const STATIC_CACHE_CONTROL: &str = "public, max-age=300, s-maxage=3600, must-revalidate";

async fn static_fallback(headers: HeaderMap, uri: Uri) -> Response {
    let raw = uri.path().trim_start_matches('/');
    let path = if raw.is_empty() { "index.html" } else { raw };

    if let Some(file) = StaticAssets::get(path) {
        return render_embed(&headers, file);
    }
    if !path.ends_with(".html")
        && let Some(file) = StaticAssets::get(&format!("{path}.html"))
    {
        return render_embed(&headers, file);
    }
    (StatusCode::NOT_FOUND, "Not Found").into_response()
}

fn render_embed(headers: &HeaderMap, file: EmbeddedFile) -> Response {
    let etag = format!("\"{}\"", hex::encode(file.metadata.sha256_hash()));

    let client_matches = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == "*" || v.split(',').any(|tag| tag.trim() == etag));

    if client_matches {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag),
                (header::CACHE_CONTROL, STATIC_CACHE_CONTROL.to_string()),
            ],
        )
            .into_response();
    }

    let mime = file.metadata.mimetype().to_owned();
    (
        [
            (header::CONTENT_TYPE, mime),
            (header::ETAG, etag),
            (header::CACHE_CONTROL, STATIC_CACHE_CONTROL.to_string()),
        ],
        file.data.into_owned(),
    )
        .into_response()
}
