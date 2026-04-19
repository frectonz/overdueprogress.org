use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("turnstile: {0}")]
    Turnstile(#[from] crate::turnstile::TurnstileError),

    #[error("resend: {0}")]
    Resend(#[from] crate::resend::ResendError),

    #[error("webauthn: {0}")]
    WebAuthn(#[from] webauthn_rs::prelude::WebauthnError),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("view: {0}")]
    View(#[from] crate::view::ViewError),

    #[error("bad request: {0}")]
    BadRequest(&'static str),

    #[error("unauthorized")]
    Unauthorized,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match &self {
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, *msg).into_response(),
            AppError::Unauthorized => {
                (StatusCode::UNAUTHORIZED, "authentication failed").into_response()
            }
            other => {
                tracing::error!(error = ?other, "handler failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
            }
        }
    }
}
