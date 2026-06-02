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
        self.send_email(Email {
            from: &self.from,
            to: [to],
            subject,
            text: Some(text),
            html: None,
        })
        .await
    }

    pub async fn send_html(&self, to: &str, subject: &str, html: &str) -> Result<(), ResendError> {
        self.send_email(Email {
            from: &self.from,
            to: [to],
            subject,
            text: None,
            html: Some(html),
        })
        .await
    }

    async fn send_email(&self, payload: Email<'_>) -> Result<(), ResendError> {
        tracing::debug!(to = %payload.to[0], subject = %payload.subject, "sending email");
        self.http
            .post(&self.send_url)
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;
        tracing::debug!(to = %payload.to[0], "email sent");
        Ok(())
    }
}

#[derive(Serialize)]
struct Email<'a> {
    from: &'a str,
    to: [&'a str; 1],
    subject: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    html: Option<&'a str>,
}
