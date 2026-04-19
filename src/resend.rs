use serde::Serialize;
use thiserror::Error;

const SEND_URL: &str = "https://api.resend.com/emails";

#[derive(Debug, Error)]
pub enum ResendError {
    #[error("http request failed: {0}")]
    Http(#[from] reqwest::Error),
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    api_key: String,
    from: String,
    send_url: String,
}

impl Client {
    pub fn new(http: reqwest::Client, api_key: String, from: String) -> Self {
        Self::with_url(http, api_key, from, SEND_URL.into())
    }

    pub fn with_url(
        http: reqwest::Client,
        api_key: String,
        from: String,
        send_url: String,
    ) -> Self {
        Self {
            http,
            api_key,
            from,
            send_url,
        }
    }

    pub async fn send(&self, to: &str, subject: &str, text: &str) -> Result<(), ResendError> {
        tracing::debug!(%to, %subject, "sending email");
        let payload = Email {
            from: &self.from,
            to: [to],
            subject,
            text,
        };
        self.http
            .post(&self.send_url)
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;
        tracing::debug!(%to, "email sent");
        Ok(())
    }
}

#[derive(Serialize)]
struct Email<'a> {
    from: &'a str,
    to: [&'a str; 1],
    subject: &'a str,
    text: &'a str,
}
