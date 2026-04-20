use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{
    Json, Router,
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use minijinja::context;
use rand::Rng;
use serde::Deserialize;
use sqlx::SqlitePool;
use tower_governor::{
    GovernorLayer, governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor,
};
use url::Url;
use uuid::Uuid;
use webauthn_rs::WebauthnBuilder;
use webauthn_rs::prelude::{
    AuthenticationResult, Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential, Webauthn,
};

use crate::AppState;
use crate::error::AppError;

#[derive(Debug, thiserror::Error)]
pub enum AuthInitError {
    #[error("invalid RP origin URL: {0}")]
    Url(#[from] url::ParseError),
    #[error("webauthn config: {0}")]
    WebAuthn(#[from] webauthn_rs::prelude::WebauthnError),
}

const ADMIN_USER_ID: Uuid = Uuid::from_u128(0xA000_0000_0000_4000_8000_0000_0000_0001);
const SESSION_COOKIE: &str = "admin_session";
const CHALLENGE_COOKIE: &str = "admin_challenge";
const CHALLENGE_TTL: Duration = Duration::from_secs(5 * 60);
const SESSION_TTL_DAYS: i64 = 7;

pub fn routes() -> Router<AppState> {
    let login_limit = GovernorLayer::new(Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(SmartIpKeyExtractor)
            .per_second(5)
            .burst_size(10)
            .finish()
            .expect("valid login rate limiter"),
    ));

    Router::new()
        .route("/admin/login", get(login_page))
        .route(
            "/admin/login/start",
            post(login_start).layer(login_limit.clone()),
        )
        .route("/admin/login/finish", post(login_finish).layer(login_limit))
        .route("/admin/logout", post(logout))
        .route("/admin/register", get(register_page))
        .route("/admin/register/start", post(register_start))
        .route("/admin/register/finish", post(register_finish))
}

type ChallengeStore = Arc<Mutex<HashMap<String, (Instant, ChallengeState)>>>;

enum ChallengeState {
    Register(PasskeyRegistration),
    Login(PasskeyAuthentication),
}

#[derive(Clone)]
pub struct Auth {
    webauthn: Arc<Webauthn>,
    challenges: ChallengeStore,
    cookies_secure: bool,
}

impl Auth {
    pub fn new(rp_id: &str, rp_origin: &str) -> Result<Self, AuthInitError> {
        let origin = Url::parse(rp_origin)?;
        let cookies_secure = origin.scheme() == "https";
        let webauthn = WebauthnBuilder::new(rp_id, &origin)?
            .rp_name("Overdue Progress Admin")
            .build()?;
        Ok(Self {
            webauthn: Arc::new(webauthn),
            challenges: Arc::new(Mutex::new(HashMap::new())),
            cookies_secure,
        })
    }
}

pub async fn require_session(state: &AppState, jar: &CookieJar) -> Option<Response> {
    if current_session(&state.db, jar).await.is_some() {
        return None;
    }
    let redirect = if passkey_count(&state.db).await == 0 {
        Redirect::to("/admin/register")
    } else {
        Redirect::to("/admin/login")
    };
    Some(redirect.into_response())
}

async fn current_session(db: &SqlitePool, jar: &CookieJar) -> Option<String> {
    let token = jar.get(SESSION_COOKIE)?.value().to_string();
    sqlx::query_scalar!(
        r#"SELECT token as "token!" FROM sessions
           WHERE token = ? AND expires_at > strftime('%Y-%m-%dT%H:%M:%SZ', 'now')"#,
        token,
    )
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

pub async fn current_csrf_token(db: &SqlitePool, jar: &CookieJar) -> Option<String> {
    let token = jar.get(SESSION_COOKIE)?.value().to_string();
    sqlx::query_scalar!(
        "SELECT csrf_token FROM sessions
         WHERE token = ? AND expires_at > strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
        token,
    )
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .flatten()
}

async fn create_session(db: &SqlitePool) -> Result<String, AppError> {
    let token = random_token();
    let csrf = random_token();
    let ttl = format!("+{SESSION_TTL_DAYS} days");
    sqlx::query!(
        "INSERT INTO sessions (token, expires_at, csrf_token)
         VALUES (?, strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?), ?)",
        token,
        ttl,
        csrf,
    )
    .execute(db)
    .await?;
    Ok(token)
}

async fn delete_session(db: &SqlitePool, token: &str) {
    let _ = sqlx::query!("DELETE FROM sessions WHERE token = ?", token)
        .execute(db)
        .await;
}

async fn passkey_count(db: &SqlitePool) -> i64 {
    sqlx::query_scalar!(r#"SELECT COUNT(*) as "count!: i64" FROM passkeys"#)
        .fetch_one(db)
        .await
        .unwrap_or(0)
}

async fn load_passkeys(db: &SqlitePool) -> Result<Vec<Passkey>, AppError> {
    let rows = sqlx::query_scalar!("SELECT data FROM passkeys")
        .fetch_all(db)
        .await?;
    rows.into_iter()
        .map(|data| serde_json::from_str(&data).map_err(AppError::from))
        .collect()
}

async fn insert_passkey(db: &SqlitePool, passkey: &Passkey, label: &str) -> Result<(), AppError> {
    let cred_id = cred_id_hex(passkey.cred_id().as_ref());
    let data = serde_json::to_string(passkey)?;
    sqlx::query!(
        "INSERT INTO passkeys (credential_id, data, label) VALUES (?, ?, ?)",
        cred_id,
        data,
        label,
    )
    .execute(db)
    .await?;
    Ok(())
}

async fn record_passkey_use(db: &SqlitePool, cred_id_bytes: &[u8], result: &AuthenticationResult) {
    let cred_id = cred_id_hex(cred_id_bytes);
    if result.needs_update() {
        let Ok(Some(data)) =
            sqlx::query_scalar!("SELECT data FROM passkeys WHERE credential_id = ?", cred_id)
                .fetch_optional(db)
                .await
        else {
            return;
        };
        let Ok(mut pk) = serde_json::from_str::<Passkey>(&data) else {
            return;
        };
        pk.update_credential(result);
        let Ok(updated) = serde_json::to_string(&pk) else {
            return;
        };
        let _ = sqlx::query!(
            "UPDATE passkeys
             SET data = ?, last_used_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE credential_id = ?",
            updated,
            cred_id,
        )
        .execute(db)
        .await;
    } else {
        let _ = sqlx::query!(
            "UPDATE passkeys
             SET last_used_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE credential_id = ?",
            cred_id,
        )
        .execute(db)
        .await;
    }
}

fn cred_id_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn put_challenge(store: &ChallengeStore, id: &str, challenge: ChallengeState) {
    let mut map = store.lock().unwrap();
    let now = Instant::now();
    map.retain(|_, (created, _)| now.duration_since(*created) < CHALLENGE_TTL);
    map.insert(id.to_string(), (now, challenge));
}

fn take_challenge(store: &ChallengeStore, id: &str) -> Option<ChallengeState> {
    let mut map = store.lock().unwrap();
    let (created, c) = map.remove(id)?;
    (Instant::now().duration_since(created) < CHALLENGE_TTL).then_some(c)
}

fn session_cookie(value: String, secure: bool) -> Cookie<'static> {
    build_cookie(
        SESSION_COOKIE,
        value,
        time::Duration::days(SESSION_TTL_DAYS),
        secure,
    )
}

fn challenge_cookie(value: String, secure: bool) -> Cookie<'static> {
    build_cookie(
        CHALLENGE_COOKIE,
        value,
        time::Duration::seconds(CHALLENGE_TTL.as_secs() as i64),
        secure,
    )
}

fn build_cookie(
    name: &'static str,
    value: String,
    max_age: time::Duration,
    secure: bool,
) -> Cookie<'static> {
    Cookie::build((name, value))
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Strict)
        .path("/")
        .max_age(max_age)
        .build()
}

fn removal_cookie(name: &'static str, secure: bool) -> Cookie<'static> {
    let mut c = Cookie::build((name, ""))
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Strict)
        .path("/")
        .build();
    c.make_removal();
    c
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    cred_id_hex(&bytes)
}

fn client_ip(headers: &HeaderMap) -> Option<String> {
    for name in ["cf-connecting-ip", "x-forwarded-for", "x-real-ip"] {
        if let Some(v) = headers.get(name).and_then(|h| h.to_str().ok()) {
            return Some(v.split(',').next().unwrap_or(v).trim().to_owned());
        }
    }
    None
}

fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_owned())
}

async fn log_auth_event(
    db: &SqlitePool,
    event: &str,
    credential_id: Option<&str>,
    headers: &HeaderMap,
) {
    let ip = client_ip(headers);
    let ua = user_agent(headers);
    if let Err(err) = sqlx::query!(
        "INSERT INTO auth_events (event, credential_id, ip, user_agent)
         VALUES (?, ?, ?, ?)",
        event,
        credential_id,
        ip,
        ua,
    )
    .execute(db)
    .await
    {
        tracing::error!(?err, event, "failed to write auth event");
    }
}

async fn may_register(state: &AppState, jar: &CookieJar) -> bool {
    current_session(&state.db, jar).await.is_some() || passkey_count(&state.db).await == 0
}

async fn login_page(State(state): State<AppState>) -> Response {
    if passkey_count(&state.db).await == 0 {
        return Redirect::to("/admin/register").into_response();
    }
    state.view.render("login.html", context! {})
}

async fn login_start(State(state): State<AppState>, jar: CookieJar) -> Result<Response, AppError> {
    let passkeys = load_passkeys(&state.db).await?;
    if passkeys.is_empty() {
        tracing::warn!("login_start: no passkeys registered");
        return Err(AppError::BadRequest("no passkeys registered"));
    }

    let (rcr, auth_state) = state
        .auth
        .webauthn
        .start_passkey_authentication(&passkeys)?;

    let id = random_token();
    put_challenge(
        &state.auth.challenges,
        &id,
        ChallengeState::Login(auth_state),
    );
    let jar = jar.add(challenge_cookie(id, state.auth.cookies_secure));
    tracing::info!(passkeys = passkeys.len(), "login challenge issued");
    Ok((jar, Json(rcr)).into_response())
}

async fn login_finish(
    State(state): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(cred): Json<PublicKeyCredential>,
) -> Result<Response, AppError> {
    let Some(id) = jar.get(CHALLENGE_COOKIE).map(|c| c.value().to_string()) else {
        tracing::warn!("login_finish: missing challenge cookie");
        return Err(AppError::BadRequest("missing challenge"));
    };
    let Some(ChallengeState::Login(auth_state)) = take_challenge(&state.auth.challenges, &id)
    else {
        tracing::warn!("login_finish: challenge expired or wrong type");
        return Err(AppError::BadRequest("challenge expired"));
    };

    let result = match state
        .auth
        .webauthn
        .finish_passkey_authentication(&cred, &auth_state)
    {
        Ok(r) => r,
        Err(err) => {
            tracing::warn!(?err, ip = ?client_ip(&headers), "login failed");
            log_auth_event(&state.db, "login_failure", None, &headers).await;
            state.telegram.notify(format!(
                "⚠️ Admin login failed (ip: {})",
                client_ip(&headers).as_deref().unwrap_or("unknown")
            ));
            return Err(AppError::Unauthorized);
        }
    };

    let cred_id_bytes: Vec<u8> = result.cred_id().as_ref().to_vec();
    let cred_id = cred_id_hex(&cred_id_bytes);
    record_passkey_use(&state.db, &cred_id_bytes, &result).await;

    let token = create_session(&state.db).await?;
    log_auth_event(&state.db, "login_success", Some(&cred_id), &headers).await;
    tracing::info!(credential_id = %cred_id, ip = ?client_ip(&headers), "login success");
    state.telegram.notify(format!(
        "🔐 Admin login (ip: {})",
        client_ip(&headers).as_deref().unwrap_or("unknown")
    ));

    let jar = jar
        .remove(removal_cookie(CHALLENGE_COOKIE, state.auth.cookies_secure))
        .add(session_cookie(token, state.auth.cookies_secure));
    Ok((jar, Json(serde_json::json!({ "ok": true }))).into_response())
}

async fn logout(State(state): State<AppState>, headers: HeaderMap, jar: CookieJar) -> Response {
    if let Some(c) = jar.get(SESSION_COOKIE) {
        delete_session(&state.db, c.value()).await;
    }
    log_auth_event(&state.db, "logout", None, &headers).await;
    tracing::info!(ip = ?client_ip(&headers), "logout");
    let jar = jar.remove(removal_cookie(SESSION_COOKIE, state.auth.cookies_secure));
    (jar, Redirect::to("/admin/login")).into_response()
}

async fn register_page(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<Response, AppError> {
    if !may_register(&state, &jar).await {
        tracing::warn!("register_page: denied");
        return Err(AppError::BadRequest(
            "Registration is closed. Sign in to add another passkey.",
        ));
    }
    Ok(state.view.render("register.html", context! {}))
}

async fn register_start(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<Response, AppError> {
    if !may_register(&state, &jar).await {
        tracing::warn!("register_start: denied");
        return Err(AppError::BadRequest("registration not allowed"));
    }

    let existing = load_passkeys(&state.db).await?;
    tracing::info!(existing = existing.len(), "register challenge issued");
    let exclude = existing.iter().map(|p| p.cred_id().clone()).collect();

    let (ccr, reg_state) = state.auth.webauthn.start_passkey_registration(
        ADMIN_USER_ID,
        "admin",
        "Administrator",
        Some(exclude),
    )?;

    let id = random_token();
    put_challenge(
        &state.auth.challenges,
        &id,
        ChallengeState::Register(reg_state),
    );
    let jar = jar.add(challenge_cookie(id, state.auth.cookies_secure));
    Ok((jar, Json(ccr)).into_response())
}

#[derive(Deserialize)]
struct RegisterLabel {
    label: Option<String>,
}

async fn register_finish(
    State(state): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
    axum::extract::Query(q): axum::extract::Query<RegisterLabel>,
    Json(cred): Json<RegisterPublicKeyCredential>,
) -> Result<Response, AppError> {
    let Some(id) = jar.get(CHALLENGE_COOKIE).map(|c| c.value().to_string()) else {
        return Err(AppError::BadRequest("missing challenge"));
    };
    let Some(ChallengeState::Register(reg_state)) = take_challenge(&state.auth.challenges, &id)
    else {
        return Err(AppError::BadRequest("challenge expired"));
    };

    let passkey = state
        .auth
        .webauthn
        .finish_passkey_registration(&cred, &reg_state)
        .map_err(|err| {
            tracing::warn!(?err, "finish_passkey_registration failed");
            AppError::BadRequest("registration failed")
        })?;

    let label = q.label.as_deref().unwrap_or("passkey");
    insert_passkey(&state.db, &passkey, label).await?;

    let cred_id = cred_id_hex(passkey.cred_id().as_ref());
    log_auth_event(&state.db, "register", Some(&cred_id), &headers).await;
    tracing::info!(credential_id = %cred_id, label, ip = ?client_ip(&headers), "passkey registered");
    state.telegram.notify(format!(
        "✨ New passkey registered\nLabel: {}\nIP: {}",
        label,
        client_ip(&headers).as_deref().unwrap_or("unknown")
    ));

    let jar = jar.remove(removal_cookie(CHALLENGE_COOKIE, state.auth.cookies_secure));
    Ok((jar, Json(serde_json::json!({ "ok": true }))).into_response())
}
