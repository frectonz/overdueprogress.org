use std::net::SocketAddr;

use parenv::Environment;

impl Env {
    pub fn load() -> Self {
        Self::parse()
    }
}

#[derive(Debug, Environment)]
pub struct Env {
    /// SQLite connection string
    #[parenv(default = "sqlite://submissions.db")]
    pub database_url: String,
    /// Cloudflare Turnstile site key (public)
    pub turnstile_site_key: String,
    /// Cloudflare Turnstile secret key
    pub turnstile_secret_key: String,
    /// Resend API key
    pub resend_api_key: String,
    /// From address for confirmation emails
    #[parenv(default = "Overdue Progress <submissions@overdueprogress.org>")]
    pub from_email: String,
    /// WebAuthn relying party ID (the site's registrable domain)
    #[parenv(default = "localhost")]
    pub rp_id: String,
    /// WebAuthn relying party origin (the full https URL, or http://localhost for dev)
    #[parenv(default = "http://localhost:3000")]
    pub rp_origin: String,
    /// Address to bind the HTTP server on
    #[parenv(default = "0.0.0.0:3000")]
    pub addr: SocketAddr,
}
