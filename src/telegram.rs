use serde::Serialize;
use thiserror::Error;

const DEFAULT_API_BASE: &str = "https://api.telegram.org";

#[derive(Debug, Error)]
pub enum TelegramError {
    #[error("http request failed: {0}")]
    Http(#[from] reqwest::Error),
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    bot_token: String,
    chat_id: String,
    api_base: String,
}

impl Client {
    pub fn new(http: reqwest::Client, bot_token: String, chat_id: String) -> Self {
        Self::with_url(http, bot_token, chat_id, DEFAULT_API_BASE.into())
    }

    pub fn with_url(
        http: reqwest::Client,
        bot_token: String,
        chat_id: String,
        api_base: String,
    ) -> Self {
        Self {
            http,
            bot_token,
            chat_id,
            api_base,
        }
    }

    /// Fire-and-forget notification; errors are logged, never propagated.
    pub fn notify(&self, text: impl Into<String>) {
        let http = self.http.clone();
        let url = format!("{}/bot{}/sendMessage", self.api_base, self.bot_token);
        let chat_id = self.chat_id.clone();
        let text = text.into();
        tokio::spawn(async move {
            if let Err(err) = send(&http, &url, &chat_id, &text).await {
                tracing::error!(?err, "telegram notify failed");
            }
        });
    }
}

async fn send(
    http: &reqwest::Client,
    url: &str,
    chat_id: &str,
    text: &str,
) -> Result<(), TelegramError> {
    let payload = SendMessage {
        chat_id,
        text,
        parse_mode: "HTML",
        disable_web_page_preview: true,
    };
    http.post(url)
        .json(&payload)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

#[derive(Serialize)]
struct SendMessage<'a> {
    chat_id: &'a str,
    text: &'a str,
    parse_mode: &'static str,
    disable_web_page_preview: bool,
}
