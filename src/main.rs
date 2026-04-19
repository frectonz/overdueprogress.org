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

use axum::{extract::DefaultBodyLimit, Router};
use axum_embed::ServeEmbed;
use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
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
        .create_if_missing(true);
    SqlitePool::connect_with(opts).await
}

fn build_router(state: AppState) -> Router {
    Router::new()
        .merge(submissions::routes())
        .merge(auth::routes())
        .fallback_service(ServeEmbed::<StaticAssets>::new())
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .with_state(state)
}
