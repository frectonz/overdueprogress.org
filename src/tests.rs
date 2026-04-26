use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use time::OffsetDateTime;
use tower::ServiceExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, path_regex},
};

use crate::{
    AppState, auth::Auth, build_router, resend, submissions, telegram, turnstile, view::View,
};

async fn setup_db() -> SqlitePool {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .create_if_missing(true);
    let db = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&db).await.unwrap();
    db
}

async fn setup_state(turnstile_ok: bool) -> (AppState, MockServer) {
    let mock = MockServer::start().await;
    let turnstile_body = if turnstile_ok {
        r#"{"success":true}"#
    } else {
        r#"{"success":false,"error-codes":["invalid-input-response"]}"#
    };
    Mock::given(method("POST"))
        .and(path("/turnstile"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(turnstile_body, "application/json"))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/resend"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(r#"{"id":"test"}"#, "application/json"),
        )
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/bot[^/]+/sendMessage$"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(r#"{"ok":true}"#, "application/json"))
        .mount(&mock)
        .await;

    let http = reqwest::Client::new();
    let state = AppState {
        db: setup_db().await,
        view: View::new(),
        turnstile: turnstile::Client::with_url(
            http.clone(),
            "site-key".into(),
            "secret-key".into(),
            format!("{}/turnstile", mock.uri()),
        ),
        resend: resend::Client::with_url(
            http.clone(),
            "resend-key".into(),
            "Test <test@example.com>".into(),
            format!("{}/resend", mock.uri()),
        ),
        telegram: telegram::Client::with_url(
            http.clone(),
            "test-token".into(),
            "test-chat".into(),
            mock.uri(),
        ),
        auth: Auth::new("localhost", "http://localhost:3000").unwrap(),
        deadline: submissions::DEADLINE,
    };
    (state, mock)
}

fn form_body(pairs: &[(&str, &str)]) -> Body {
    Body::from(serde_urlencoded::to_string(pairs).unwrap())
}

const TEST_IP: &str = "127.0.0.1";

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header("x-forwarded-for", TEST_IP)
        .body(Body::empty())
        .unwrap()
}

fn submit_request(
    title: &str,
    description: &str,
    author: &str,
    email: &str,
    link: &str,
) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/submit")
        .header("content-type", "application/x-www-form-urlencoded")
        .header("x-forwarded-for", TEST_IP)
        .body(form_body(&[
            ("title", title),
            ("description", description),
            ("author", author),
            ("email", email),
            ("link", link),
            ("cf-turnstile-response", "dummy"),
        ]))
        .unwrap()
}

async fn body_text(res: axum::response::Response) -> String {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn index_is_served() {
    let (state, _mock) = setup_state(true).await;
    let app = build_router(state);

    let res = app.oneshot(get("/")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(body_text(res).await.contains("Overdue Progress"));
}

#[tokio::test]
async fn submit_page_renders() {
    let (state, _mock) = setup_state(true).await;
    let app = build_router(state);

    let res = app.oneshot(get("/submit")).await.unwrap();
    let status = res.status();
    let body = body_text(res).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body.contains("Submit"));
    assert!(body.contains("site-key")); // turnstile site key rendered
}

#[tokio::test]
async fn submit_happy_path_stores_row() {
    let (state, _mock) = setup_state(true).await;
    let db = state.db.clone();
    let app = build_router(state);

    let res = app
        .oneshot(submit_request(
            "My essay",
            "A thoughtful description of progress.",
            "Alice",
            "alice@example.com",
            "https://example.com/essay",
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(body_text(res).await.contains("received"));

    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM submissions")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn submit_sends_html_telegram_notification() {
    let (state, mock) = setup_state(true).await;
    let app = build_router(state);

    let res = app
        .oneshot(submit_request(
            "My essay",
            "A thoughtful description of progress.",
            "Alice <script>",
            "alice@example.com",
            "https://example.com/essay",
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Telegram is fire-and-forget; let the spawned task reach the mock.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let requests = mock.received_requests().await.unwrap();
    let tg = requests
        .iter()
        .find(|r| r.url.path().ends_with("/sendMessage"))
        .expect("telegram sendMessage was called");
    let body: serde_json::Value = serde_json::from_slice(&tg.body).unwrap();
    assert_eq!(body["parse_mode"], "HTML");
    let text = body["text"].as_str().unwrap();
    assert!(text.contains("<b>"), "expected bold tag in: {text}");
    assert!(
        text.contains("Alice &lt;script&gt;"),
        "expected escaped author in: {text}"
    );
}

#[tokio::test]
async fn submit_rejects_bad_email() {
    let (state, _mock) = setup_state(true).await;
    let db = state.db.clone();
    let app = build_router(state);

    let res = app
        .oneshot(submit_request(
            "t",
            "d",
            "a",
            "not-an-email",
            "https://example.com",
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(
        body_text(res)
            .await
            .contains("email address doesn&#x27;t look valid")
    );

    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM submissions")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn submit_rejects_turnstile_failure() {
    let (state, _mock) = setup_state(false).await;
    let db = state.db.clone();
    let app = build_router(state);

    let res = app
        .oneshot(submit_request(
            "t",
            "d",
            "a",
            "a@b.co",
            "https://example.com",
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(body_text(res).await.contains("Bot check failed"));

    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM submissions")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn submit_rejects_oversized_body() {
    let (state, _mock) = setup_state(true).await;
    let app = build_router(state);

    let huge = "x".repeat(80_000);
    let res = app
        .oneshot(submit_request(
            &huge,
            "d",
            "a",
            "a@b.co",
            "https://example.com",
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn submit_page_shows_closed_after_deadline() {
    let (mut state, _mock) = setup_state(true).await;
    state.deadline = OffsetDateTime::UNIX_EPOCH;
    let app = build_router(state);

    let res = app.oneshot(get("/submit")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_text(res).await;
    assert!(body.contains("Submissions closed"), "body: {body}");
    assert!(!body.contains("<form"), "form should be hidden");
}

#[tokio::test]
async fn submit_rejected_after_deadline() {
    let (mut state, _mock) = setup_state(true).await;
    state.deadline = OffsetDateTime::UNIX_EPOCH;
    let db = state.db.clone();
    let app = build_router(state);

    let res = app
        .oneshot(submit_request(
            "t",
            "d",
            "a",
            "a@b.co",
            "https://example.com",
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_text(res).await;
    assert!(body.contains("Sorry you missed it"), "body: {body}");
    assert!(!body.contains("<form"), "form should not render");

    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM submissions")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn deadline_page_shows_countdown_before_deadline() {
    let (state, _mock) = setup_state(true).await;
    let app = build_router(state);

    let res = app.oneshot(get("/deadline")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_text(res).await;
    assert!(body.contains("countdown"), "expected countdown div");
    assert!(!body.contains("Submissions are closed"));
}

#[tokio::test]
async fn deadline_page_shows_closed_after_deadline() {
    let (mut state, _mock) = setup_state(true).await;
    state.deadline = OffsetDateTime::UNIX_EPOCH;
    let app = build_router(state);

    let res = app.oneshot(get("/deadline")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_text(res).await;
    assert!(body.contains("Submissions are closed"), "body: {body}");
    assert!(
        !body.contains(r#"id="countdown""#),
        "countdown should be hidden"
    );
}

#[tokio::test]
async fn admin_redirects_to_register_when_empty() {
    let (state, _mock) = setup_state(true).await;
    let app = build_router(state);

    let res = app.oneshot(get("/admin")).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(res.headers().get("location").unwrap(), "/admin/register");
}

#[tokio::test]
async fn login_page_redirects_when_no_passkeys() {
    let (state, _mock) = setup_state(true).await;
    let app = build_router(state);

    let res = app.oneshot(get("/admin/login")).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn register_allowed_when_empty() {
    let (state, _mock) = setup_state(true).await;
    let app = build_router(state);

    let res = app.oneshot(get("/admin/register")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn register_forbidden_when_passkey_exists() {
    let (state, _mock) = setup_state(true).await;
    sqlx::query("INSERT INTO passkeys (credential_id, data, label) VALUES ('x', '{}', 'seed')")
        .execute(&state.db)
        .await
        .unwrap();
    let app = build_router(state);

    let res = app.oneshot(get("/admin/register")).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn delete_rejects_bad_csrf_token() {
    let (state, _mock) = setup_state(true).await;
    sqlx::query(
        "INSERT INTO submissions (title, description, author, email, link)
         VALUES ('t', 'd', 'a', 'a@b.co', 'https://a.b')",
    )
    .execute(&state.db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO sessions (token, expires_at, csrf_token)
         VALUES ('sess', strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '+1 day'), 'good-csrf')",
    )
    .execute(&state.db)
    .await
    .unwrap();
    let db = state.db.clone();
    let app = build_router(state);

    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/submissions/1/delete")
                .header("content-type", "application/x-www-form-urlencoded")
                .header("x-forwarded-for", TEST_IP)
                .header("cookie", "admin_session=sess")
                .body(form_body(&[("csrf_token", "wrong")]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM submissions")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(count, 1, "row should still exist after rejected delete");
}

#[tokio::test]
async fn delete_succeeds_with_valid_csrf_token() {
    let (state, _mock) = setup_state(true).await;
    sqlx::query(
        "INSERT INTO submissions (title, description, author, email, link)
         VALUES ('t', 'd', 'a', 'a@b.co', 'https://a.b')",
    )
    .execute(&state.db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO sessions (token, expires_at, csrf_token)
         VALUES ('sess', strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '+1 day'), 'good-csrf')",
    )
    .execute(&state.db)
    .await
    .unwrap();
    let db = state.db.clone();
    let app = build_router(state);

    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/submissions/1/delete")
                .header("content-type", "application/x-www-form-urlencoded")
                .header("x-forwarded-for", TEST_IP)
                .header("cookie", "admin_session=sess")
                .body(form_body(&[("csrf_token", "good-csrf")]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM submissions")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

async fn seed_submission_and_session(db: &SqlitePool) {
    sqlx::query(
        "INSERT INTO submissions (title, description, author, email, link)
         VALUES ('orig title', 'orig desc', 'orig author', 'orig@b.co', 'https://orig.example')",
    )
    .execute(db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO sessions (token, expires_at, csrf_token)
         VALUES ('sess', strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '+1 day'), 'good-csrf')",
    )
    .execute(db)
    .await
    .unwrap();
}

fn edit_form_body(csrf: &str) -> Body {
    form_body(&[
        ("csrf_token", csrf),
        ("title", "new title"),
        ("description", "new description"),
        ("author", "new author"),
        ("email", "new@b.co"),
        ("link", "https://new.example"),
    ])
}

#[tokio::test]
async fn edit_rejects_bad_csrf_token() {
    let (state, _mock) = setup_state(true).await;
    seed_submission_and_session(&state.db).await;
    let db = state.db.clone();
    let app = build_router(state);

    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/submissions/1/edit")
                .header("content-type", "application/x-www-form-urlencoded")
                .header("x-forwarded-for", TEST_IP)
                .header("cookie", "admin_session=sess")
                .body(edit_form_body("wrong"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let title: (String,) = sqlx::query_as("SELECT title FROM submissions WHERE id = 1")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(title.0, "orig title");

    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM submission_edits")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn edit_updates_submission_and_records_history() {
    let (state, _mock) = setup_state(true).await;
    seed_submission_and_session(&state.db).await;
    let db = state.db.clone();
    let app = build_router(state);

    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/submissions/1/edit")
                .header("content-type", "application/x-www-form-urlencoded")
                .header("x-forwarded-for", TEST_IP)
                .header("cookie", "admin_session=sess")
                .body(edit_form_body("good-csrf"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    let updated: (String, String, String, String, String) = sqlx::query_as(
        "SELECT title, description, author, email, link FROM submissions WHERE id = 1",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(updated.0, "new title");
    assert_eq!(updated.1, "new description");
    assert_eq!(updated.2, "new author");
    assert_eq!(updated.3, "new@b.co");
    assert_eq!(updated.4, "https://new.example");

    let snap: (String, String, String) = sqlx::query_as(
        "SELECT title, edit_kind, author FROM submission_edits WHERE submission_id = 1",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(snap.0, "orig title");
    assert_eq!(snap.1, "edit");
    assert_eq!(snap.2, "orig author");
}

#[tokio::test]
async fn edit_with_identical_values_writes_no_history() {
    let (state, _mock) = setup_state(true).await;
    seed_submission_and_session(&state.db).await;
    let db = state.db.clone();
    let app = build_router(state);

    let body = form_body(&[
        ("csrf_token", "good-csrf"),
        ("title", "orig title"),
        ("description", "orig desc"),
        ("author", "orig author"),
        ("email", "orig@b.co"),
        ("link", "https://orig.example"),
    ]);
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/submissions/1/edit")
                .header("content-type", "application/x-www-form-urlencoded")
                .header("x-forwarded-for", TEST_IP)
                .header("cookie", "admin_session=sess")
                .body(body)
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM submission_edits")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(count, 0, "no-op edit should not write history");
}

#[tokio::test]
async fn revert_restores_prior_values_and_logs_revert_snapshot() {
    let (state, _mock) = setup_state(true).await;
    seed_submission_and_session(&state.db).await;
    let db = state.db.clone();
    let app = build_router(state);

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/submissions/1/edit")
                .header("content-type", "application/x-www-form-urlencoded")
                .header("x-forwarded-for", TEST_IP)
                .header("cookie", "admin_session=sess")
                .body(edit_form_body("good-csrf"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    let (edit_id,): (i64,) =
        sqlx::query_as("SELECT id FROM submission_edits WHERE submission_id = 1")
            .fetch_one(&db)
            .await
            .unwrap();

    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/admin/submissions/1/history/{edit_id}/revert"))
                .header("content-type", "application/x-www-form-urlencoded")
                .header("x-forwarded-for", TEST_IP)
                .header("cookie", "admin_session=sess")
                .body(form_body(&[("csrf_token", "good-csrf")]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    let restored: (String, String) =
        sqlx::query_as("SELECT title, author FROM submissions WHERE id = 1")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(restored.0, "orig title");
    assert_eq!(restored.1, "orig author");

    let revert_snap: (String, String, Option<i64>) = sqlx::query_as(
        "SELECT title, edit_kind, reverted_from FROM submission_edits
         WHERE submission_id = 1 ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(revert_snap.0, "new title");
    assert_eq!(revert_snap.1, "revert");
    assert_eq!(revert_snap.2, Some(edit_id));
}

#[tokio::test]
async fn stats_redirects_when_unauthenticated() {
    let (state, _mock) = setup_state(true).await;
    sqlx::query("INSERT INTO passkeys (credential_id, data, label) VALUES ('x', '{}', 'seed')")
        .execute(&state.db)
        .await
        .unwrap();
    let app = build_router(state);

    let res = app.oneshot(get("/admin/stats")).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(res.headers().get("location").unwrap(), "/admin/login");
}

#[tokio::test]
async fn stats_renders_with_data() {
    let (state, _mock) = setup_state(true).await;
    // Two submissions from different authors and link domains, with a few
    // shared description words so the cloud has something to weight.
    sqlx::query(
        "INSERT INTO submissions (title, description, author, email, link)
         VALUES
            ('Progress notes', 'progress means slow steady steady work today', 'Alice', 'alice@example.com', 'https://example.com/a'),
            ('Slow progress', 'slow steady progress overdue today work matters', 'Bob', 'bob@gmail.com', 'https://substack.com/b')",
    )
    .execute(&state.db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO sessions (token, expires_at, csrf_token)
         VALUES ('sess', strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '+1 day'), 'good-csrf')",
    )
    .execute(&state.db)
    .await
    .unwrap();
    let app = build_router(state);

    let res = app
        .oneshot(
            Request::builder()
                .uri("/admin/stats")
                .header("x-forwarded-for", TEST_IP)
                .header("cookie", "admin_session=sess")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let body = body_text(res).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body.contains("Stats"));
    // Frequency-weighted words from the two descriptions.
    assert!(body.contains("progress"), "expected progress in cloud");
    assert!(body.contains("steady"), "expected steady in cloud");
    // Email and link domains are aggregated.
    assert!(body.contains("example.com"));
    assert!(body.contains("substack.com"));
    assert!(body.contains("gmail.com"));
}

#[tokio::test]
async fn stats_renders_empty_state() {
    let (state, _mock) = setup_state(true).await;
    sqlx::query(
        "INSERT INTO sessions (token, expires_at, csrf_token)
         VALUES ('sess', strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '+1 day'), 'good-csrf')",
    )
    .execute(&state.db)
    .await
    .unwrap();
    let app = build_router(state);

    let res = app
        .oneshot(
            Request::builder()
                .uri("/admin/stats")
                .header("x-forwarded-for", TEST_IP)
                .header("cookie", "admin_session=sess")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_text(res).await;
    assert!(body.contains("No submissions yet"), "body: {body}");
}

#[tokio::test]
async fn admin_rejects_unknown_session_cookie() {
    let (state, _mock) = setup_state(true).await;
    sqlx::query("INSERT INTO passkeys (credential_id, data, label) VALUES ('x', '{}', 'seed')")
        .execute(&state.db)
        .await
        .unwrap();
    let app = build_router(state);

    let res = app
        .oneshot(
            Request::builder()
                .uri("/admin")
                .header("x-forwarded-for", TEST_IP)
                .header("cookie", "admin_session=not-a-real-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(res.headers().get("location").unwrap(), "/admin/login");
}
