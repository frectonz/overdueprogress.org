use serde::{Deserialize, Serialize};
use thiserror::Error;

const VERIFY_URL: &str = "https://challenges.cloudflare.com/turnstile/v0/siteverify";

#[derive(Debug, Error)]
pub enum TurnstileError {
    #[error("http request failed: {0}")]
    Http(#[from] reqwest::Error),
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    pub site_key: String,
    secret_key: String,
}

impl Client {
    pub fn new(http: reqwest::Client, site_key: String, secret_key: String) -> Self {
        Self {
            http,
            site_key,
            secret_key,
        }
    }

    pub async fn verify(&self, token: &str) -> Result<bool, TurnstileError> {
        tracing::debug!(token_len = token.len(), "turnstile verify");
        let req = VerifyRequest {
            secret: &self.secret_key,
            response: token,
        };
        let res = self
            .http
            .post(VERIFY_URL)
            .form(&req)
            .send()
            .await?
            .error_for_status()?
            .json::<VerifyResponse>()
            .await?;
        if res.success {
            tracing::debug!("turnstile verification ok");
        } else {
            tracing::warn!(?res.error_codes, "turnstile verification failed");
        }
        Ok(res.success)
    }
}

#[derive(Serialize)]
struct VerifyRequest<'a> {
    secret: &'a str,
    response: &'a str,
}

#[derive(Deserialize)]
struct VerifyResponse {
    success: bool,
    #[serde(rename = "error-codes", default)]
    error_codes: Vec<String>,
}
