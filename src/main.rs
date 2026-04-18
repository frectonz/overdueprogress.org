mod auth;
mod config;
mod error;
mod resend;
mod submissions;
mod turnstile;
mod view;

use std::net::SocketAddr;

use axum::{extract::DefaultBodyLimit, Router};
use axum_embed::ServeEmbed;
use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
use tower_http::trace::TraceLayer;

use crate::auth::Auth;
use crate::config::Env;
use crate::view::{StaticAssets, View};

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub view: View,
    pub turnstile: turnstile::Client,
    pub resend: resend::Client,
    pub auth: Auth,
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=info".into()),
        )
        .init();

    let env = Env::load();
    let db = connect_db(&env.database_url).await?;
    sqlx::migrate!("./migrations").run(&db).await?;

    let http = reqwest::Client::new();
    let state = AppState {
        db,
        view: View::new(),
        turnstile: turnstile::Client::new(
            http.clone(),
            env.turnstile_site_key,
            env.turnstile_secret_key,
        ),
        resend: resend::Client::new(http.clone(), env.resend_api_key, env.from_email),
        auth: Auth::new(&env.rp_id, &env.rp_origin)?,
    };

    let app = Router::new()
        .merge(submissions::routes())
        .merge(auth::routes())
        .fallback_service(ServeEmbed::<StaticAssets>::new())
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

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
