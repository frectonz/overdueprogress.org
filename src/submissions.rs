use std::sync::Arc;

use axum::{
    Form, Router,
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use axum_extra::extract::cookie::CookieJar;
use minijinja::{Value, context};
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, macros::datetime};
use tower_governor::{
    GovernorLayer, governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor,
};

use crate::AppState;
use crate::auth;
use crate::error::AppError;

pub const DEADLINE: OffsetDateTime = datetime!(2026-04-26 20:59:00 UTC);

pub fn routes() -> Router<AppState> {
    let submit_limit = GovernorLayer::new(Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(SmartIpKeyExtractor)
            .per_second(30)
            .burst_size(5)
            .finish()
            .expect("valid submit rate limiter"),
    ));

    let delete_limit = GovernorLayer::new(Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(SmartIpKeyExtractor)
            .per_second(2)
            .burst_size(5)
            .finish()
            .expect("valid delete rate limiter"),
    ));

    let edit_limit = GovernorLayer::new(Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(SmartIpKeyExtractor)
            .per_second(2)
            .burst_size(5)
            .finish()
            .expect("valid edit rate limiter"),
    ));

    Router::new()
        .route(
            "/submit",
            get(submit_page).post(submit_handler).layer(submit_limit),
        )
        .route("/deadline", get(deadline_page))
        .route("/admin", get(admin_page))
        .route(
            "/admin/submissions/{id}/delete",
            post(delete_submission).layer(delete_limit),
        )
        .route(
            "/admin/submissions/{id}/edit",
            get(edit_page).post(edit_handler).layer(edit_limit.clone()),
        )
        .route("/admin/submissions/{id}/history", get(history_page))
        .route(
            "/admin/submissions/{id}/history/{edit_id}/revert",
            post(revert_submission).layer(edit_limit),
        )
}

async fn deadline_page(State(state): State<AppState>) -> Result<Response, AppError> {
    let count = sqlx::query_scalar!("SELECT COUNT(*) FROM submissions")
        .fetch_one(&state.db)
        .await?;
    Ok(state.view.render(
        "deadline.html",
        context! {
            submission_count => count,
            closed => state.submissions_closed(),
        },
    ))
}

#[derive(Clone, Default, Serialize)]
struct FormValues {
    title: String,
    description: String,
    author: String,
    email: String,
    link: String,
}

impl FormValues {
    fn from_submitted(form: &SubmitForm) -> Self {
        Self {
            title: form.title.trim().to_owned(),
            description: form.description.trim().to_owned(),
            author: form.author.trim().to_owned(),
            email: form.email.trim().to_owned(),
            link: form.link.trim().to_owned(),
        }
    }

    fn is_complete(&self) -> bool {
        !self.title.is_empty()
            && !self.description.is_empty()
            && !self.author.is_empty()
            && !self.email.is_empty()
            && !self.link.is_empty()
    }
}

#[derive(Deserialize)]
struct SubmitForm {
    title: String,
    description: String,
    author: String,
    email: String,
    link: String,
    #[serde(rename = "cf-turnstile-response", default)]
    turnstile_response: String,
}

async fn submit_page(State(state): State<AppState>) -> Response {
    render_form(&state, &FormValues::default(), None)
}

async fn submit_handler(State(state): State<AppState>, Form(form): Form<SubmitForm>) -> Response {
    if state.submissions_closed() {
        tracing::warn!("submission rejected: deadline passed");
        return render_form(
            &state,
            &FormValues::from_submitted(&form),
            Some("Submissions are closed."),
        );
    }

    let values = FormValues::from_submitted(&form);
    tracing::info!(
        author = %values.author,
        email = %values.email,
        link = %values.link,
        title_len = values.title.chars().count(),
        description_len = values.description.chars().count(),
        "submission received"
    );

    if let Some(msg) = validate(&values) {
        tracing::warn!(reason = msg, "submission rejected: validation");
        return render_form(&state, &values, Some(msg));
    }

    match state.turnstile.verify(&form.turnstile_response).await {
        Ok(true) => tracing::debug!("turnstile ok"),
        Ok(false) => {
            tracing::warn!("submission rejected: turnstile challenge failed");
            return render_form(
                &state,
                &values,
                Some("Bot check failed. Please retry the challenge and submit again."),
            );
        }
        Err(err) => {
            tracing::error!(?err, "turnstile verify errored");
            return render_form(
                &state,
                &values,
                Some("We couldn't verify the bot check. Please try again."),
            );
        }
    }

    if let Err(err) = insert_submission(&state, &values).await {
        tracing::error!(?err, author = %values.author, "insert submission failed");
        state.notify_telegram(
            "telegram/submission_insert_failed.tg.html",
            context! {
                author => &values.author,
                email => &values.email,
                error => err.to_string(),
            },
        );
        return render_form(
            &state,
            &values,
            Some("Something went wrong on our end. Please try again."),
        );
    }
    tracing::info!(author = %values.author, email = %values.email, "submission stored");
    state.notify_telegram(
        "telegram/submission_new.tg.html",
        context! {
            author => &values.author,
            email => &values.email,
            title => &values.title,
            link => &values.link,
        },
    );

    match send_confirmation(&state, &values).await {
        Ok(_) => tracing::info!(email = %values.email, "confirmation email sent"),
        Err(err) => {
            tracing::error!(
                ?err,
                email = %values.email,
                "resend send failed (submission already saved)"
            );
            state.notify_telegram(
                "telegram/confirmation_email_failed.tg.html",
                context! {
                    email => &values.email,
                    error => err.to_string(),
                },
            );
        }
    }

    state.view.render("success.html", context! {})
}

fn render_form(state: &AppState, values: &FormValues, flash: Option<&str>) -> Response {
    state.view.render(
        "submit.html",
        context! {
            turnstile_site_key => &state.turnstile.site_key,
            values => values,
            flash => flash,
            closed => state.submissions_closed(),
        },
    )
}

const MAX_TITLE: usize = 200;
const MAX_DESCRIPTION: usize = 2000;
const MAX_AUTHOR: usize = 200;
const MAX_EMAIL: usize = 320;
const MAX_LINK: usize = 2000;

fn validate(values: &FormValues) -> Option<&'static str> {
    if !values.is_complete() {
        return Some("Please fill in every field.");
    }
    if values.title.chars().count() > MAX_TITLE {
        return Some("Title is too long.");
    }
    if values.description.chars().count() > MAX_DESCRIPTION {
        return Some("Description is too long.");
    }
    if values.author.chars().count() > MAX_AUTHOR {
        return Some("Author name is too long.");
    }
    if values.email.chars().count() > MAX_EMAIL {
        return Some("Email is too long.");
    }
    if values.link.chars().count() > MAX_LINK {
        return Some("Link is too long.");
    }
    if !valid_email(&values.email) {
        return Some("That email address doesn't look valid.");
    }
    if !valid_url(&values.link) {
        return Some("The essay link must be a full http(s) URL.");
    }
    None
}

fn valid_url(s: &str) -> bool {
    let s = s.trim();
    (s.starts_with("http://") || s.starts_with("https://")) && s.len() >= 10
}

fn valid_email(s: &str) -> bool {
    let s = s.trim();
    let Some((local, domain)) = s.split_once('@') else {
        return false;
    };
    !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
}

async fn insert_submission(state: &AppState, values: &FormValues) -> Result<(), AppError> {
    sqlx::query!(
        "INSERT INTO submissions (title, description, author, email, link) VALUES (?, ?, ?, ?, ?)",
        values.title,
        values.description,
        values.author,
        values.email,
        values.link,
    )
    .execute(&state.db)
    .await?;
    Ok(())
}

async fn send_confirmation(state: &AppState, values: &FormValues) -> Result<(), AppError> {
    let text = state.view.render_to_string(
        "confirmation_email.txt",
        context! {
            author => &values.author,
            title => &values.title,
            link => &values.link,
        },
    )?;
    state
        .resend
        .send(&values.email, "Your Overdue Progress submission", &text)
        .await?;
    Ok(())
}

#[derive(Serialize)]
struct SubmissionRow {
    id: i64,
    title: String,
    description: String,
    author: String,
    email: String,
    link: String,
    created_at: String,
}

async fn admin_page(State(state): State<AppState>, jar: CookieJar) -> Result<Response, AppError> {
    if let Some(redirect) = auth::require_session(&state, &jar).await {
        tracing::debug!("admin page accessed without session; redirecting");
        return Ok(redirect);
    }

    let csrf_token = auth::current_csrf_token(&state.db, &jar).await;

    let rows = sqlx::query_as!(
        SubmissionRow,
        "SELECT id, title, description, author, email, link, created_at
         FROM submissions ORDER BY id DESC",
    )
    .fetch_all(&state.db)
    .await?;

    tracing::info!(count = rows.len(), "admin page rendered");
    Ok(state
        .view
        .render("admin.html", admin_context(&rows, csrf_token.as_deref())))
}

fn admin_context(rows: &[SubmissionRow], csrf_token: Option<&str>) -> Value {
    context! { rows => rows, csrf_token => csrf_token }
}

#[derive(Deserialize)]
struct DeleteForm {
    csrf_token: String,
}

async fn delete_submission(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<i64>,
    Form(form): Form<DeleteForm>,
) -> Result<Response, AppError> {
    if let Some(redirect) = auth::require_session(&state, &jar).await {
        tracing::debug!(id, "delete submission denied: no session");
        return Ok(redirect);
    }

    let Some(expected) = auth::current_csrf_token(&state.db, &jar).await else {
        tracing::warn!(id, "delete submission denied: no csrf token on session");
        return Err(AppError::BadRequest("invalid csrf token"));
    };
    if form.csrf_token != expected {
        tracing::warn!(id, "delete submission denied: csrf token mismatch");
        return Err(AppError::BadRequest("invalid csrf token"));
    }

    let row = sqlx::query!(
        "SELECT title, author, email, link FROM submissions WHERE id = ?",
        id
    )
    .fetch_optional(&state.db)
    .await?;

    let Some(row) = row else {
        tracing::warn!(id, "delete submission: no such row");
        return Ok(Redirect::to("/admin").into_response());
    };

    sqlx::query!("DELETE FROM submissions WHERE id = ?", id)
        .execute(&state.db)
        .await?;

    tracing::info!(id, "submission deleted");
    state.notify_telegram(
        "telegram/submission_deleted.tg.html",
        context! {
            author => &row.author,
            email => &row.email,
            title => &row.title,
            link => &row.link,
        },
    );
    Ok(Redirect::to("/admin").into_response())
}

#[derive(Serialize)]
struct EditRow {
    id: i64,
    title: String,
    description: String,
    author: String,
    email: String,
    link: String,
    created_at: String,
    edit_kind: String,
    reverted_from: Option<i64>,
}

#[derive(Deserialize)]
struct EditSubmissionForm {
    csrf_token: String,
    title: String,
    description: String,
    author: String,
    email: String,
    link: String,
}

async fn edit_page(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    if let Some(redirect) = auth::require_session(&state, &jar).await {
        return Ok(redirect);
    }
    let csrf_token = auth::current_csrf_token(&state.db, &jar).await;

    let row = sqlx::query!(
        "SELECT title, description, author, email, link FROM submissions WHERE id = ?",
        id
    )
    .fetch_optional(&state.db)
    .await?;

    let Some(row) = row else {
        tracing::warn!(id, "edit page: no such submission");
        return Ok(Redirect::to("/admin").into_response());
    };

    let values = FormValues {
        title: row.title,
        description: row.description,
        author: row.author,
        email: row.email,
        link: row.link,
    };
    Ok(render_edit_form(
        &state,
        id,
        &values,
        None,
        csrf_token.as_deref(),
    ))
}

async fn edit_handler(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<i64>,
    Form(form): Form<EditSubmissionForm>,
) -> Result<Response, AppError> {
    if let Some(redirect) = auth::require_session(&state, &jar).await {
        tracing::debug!(id, "edit submission denied: no session");
        return Ok(redirect);
    }

    let Some(expected) = auth::current_csrf_token(&state.db, &jar).await else {
        tracing::warn!(id, "edit submission denied: no csrf token on session");
        return Err(AppError::BadRequest("invalid csrf token"));
    };
    if form.csrf_token != expected {
        tracing::warn!(id, "edit submission denied: csrf token mismatch");
        return Err(AppError::BadRequest("invalid csrf token"));
    }

    let values = FormValues {
        title: form.title.trim().to_owned(),
        description: form.description.trim().to_owned(),
        author: form.author.trim().to_owned(),
        email: form.email.trim().to_owned(),
        link: form.link.trim().to_owned(),
    };

    if let Some(msg) = validate(&values) {
        tracing::warn!(id, reason = msg, "edit submission rejected: validation");
        return Ok(render_edit_form(
            &state,
            id,
            &values,
            Some(msg),
            Some(&expected),
        ));
    }

    let mut tx = state.db.begin().await?;
    let current = sqlx::query!(
        "SELECT title, description, author, email, link FROM submissions WHERE id = ?",
        id
    )
    .fetch_optional(&mut *tx)
    .await?;

    let Some(current) = current else {
        tracing::warn!(id, "edit submission: no such row");
        return Ok(Redirect::to("/admin").into_response());
    };

    if current.title == values.title
        && current.description == values.description
        && current.author == values.author
        && current.email == values.email
        && current.link == values.link
    {
        tracing::info!(id, "edit submission: no-op, skipping");
        return Ok(Redirect::to("/admin").into_response());
    }

    sqlx::query!(
        "INSERT INTO submission_edits
            (submission_id, title, description, author, email, link, edit_kind)
         VALUES (?, ?, ?, ?, ?, ?, 'edit')",
        id,
        current.title,
        current.description,
        current.author,
        current.email,
        current.link,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE submissions
            SET title = ?, description = ?, author = ?, email = ?, link = ?
            WHERE id = ?",
        values.title,
        values.description,
        values.author,
        values.email,
        values.link,
        id,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    tracing::info!(id, "submission edited");
    state.notify_telegram(
        "telegram/submission_edited.tg.html",
        context! {
            id => id,
            author => &values.author,
            email => &values.email,
            title => &values.title,
            link => &values.link,
        },
    );
    Ok(Redirect::to("/admin").into_response())
}

async fn history_page(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    if let Some(redirect) = auth::require_session(&state, &jar).await {
        return Ok(redirect);
    }
    let csrf_token = auth::current_csrf_token(&state.db, &jar).await;

    let row = sqlx::query_as!(
        SubmissionRow,
        "SELECT id, title, description, author, email, link, created_at
         FROM submissions WHERE id = ?",
        id,
    )
    .fetch_optional(&state.db)
    .await?;

    let Some(row) = row else {
        tracing::warn!(id, "history page: no such submission");
        return Ok(Redirect::to("/admin").into_response());
    };

    let edits = sqlx::query_as!(
        EditRow,
        r#"SELECT id as "id!", title, description, author, email, link, created_at,
                  edit_kind, reverted_from
           FROM submission_edits
           WHERE submission_id = ?
           ORDER BY id DESC"#,
        id,
    )
    .fetch_all(&state.db)
    .await?;

    tracing::info!(id, count = edits.len(), "history page rendered");
    Ok(state.view.render(
        "history.html",
        context! {
            row => row,
            edits => edits,
            csrf_token => csrf_token,
        },
    ))
}

async fn revert_submission(
    State(state): State<AppState>,
    jar: CookieJar,
    Path((id, edit_id)): Path<(i64, i64)>,
    Form(form): Form<DeleteForm>,
) -> Result<Response, AppError> {
    if let Some(redirect) = auth::require_session(&state, &jar).await {
        tracing::debug!(id, edit_id, "revert denied: no session");
        return Ok(redirect);
    }

    let Some(expected) = auth::current_csrf_token(&state.db, &jar).await else {
        tracing::warn!(id, edit_id, "revert denied: no csrf token on session");
        return Err(AppError::BadRequest("invalid csrf token"));
    };
    if form.csrf_token != expected {
        tracing::warn!(id, edit_id, "revert denied: csrf token mismatch");
        return Err(AppError::BadRequest("invalid csrf token"));
    }

    let mut tx = state.db.begin().await?;
    let snapshot = sqlx::query!(
        "SELECT title, description, author, email, link FROM submission_edits
         WHERE id = ? AND submission_id = ?",
        edit_id,
        id,
    )
    .fetch_optional(&mut *tx)
    .await?;

    let Some(snapshot) = snapshot else {
        tracing::warn!(id, edit_id, "revert: no such history entry");
        return Ok(Redirect::to(&format!("/admin/submissions/{id}/history")).into_response());
    };

    let current = sqlx::query!(
        "SELECT title, description, author, email, link FROM submissions WHERE id = ?",
        id,
    )
    .fetch_optional(&mut *tx)
    .await?;

    let Some(current) = current else {
        tracing::warn!(id, edit_id, "revert: no such submission");
        return Ok(Redirect::to("/admin").into_response());
    };

    if current.title == snapshot.title
        && current.description == snapshot.description
        && current.author == snapshot.author
        && current.email == snapshot.email
        && current.link == snapshot.link
    {
        tracing::info!(id, edit_id, "revert: already at this state, skipping");
        return Ok(Redirect::to(&format!("/admin/submissions/{id}/history")).into_response());
    }

    sqlx::query!(
        "INSERT INTO submission_edits
            (submission_id, title, description, author, email, link,
             edit_kind, reverted_from)
         VALUES (?, ?, ?, ?, ?, ?, 'revert', ?)",
        id,
        current.title,
        current.description,
        current.author,
        current.email,
        current.link,
        edit_id,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE submissions
            SET title = ?, description = ?, author = ?, email = ?, link = ?
            WHERE id = ?",
        snapshot.title,
        snapshot.description,
        snapshot.author,
        snapshot.email,
        snapshot.link,
        id,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    tracing::info!(id, edit_id, "submission reverted");
    state.notify_telegram(
        "telegram/submission_reverted.tg.html",
        context! {
            id => id,
            edit_id => edit_id,
            title => &snapshot.title,
            author => &snapshot.author,
            email => &snapshot.email,
            link => &snapshot.link,
        },
    );
    Ok(Redirect::to(&format!("/admin/submissions/{id}/history")).into_response())
}

fn render_edit_form(
    state: &AppState,
    id: i64,
    values: &FormValues,
    flash: Option<&str>,
    csrf_token: Option<&str>,
) -> Response {
    state.view.render(
        "edit.html",
        context! {
            id => id,
            values => values,
            flash => flash,
            csrf_token => csrf_token,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_validation() {
        assert!(valid_email("a@b.co"));
        assert!(valid_email("  first.last+tag@example.com  "));
        assert!(!valid_email(""));
        assert!(!valid_email("no-at"));
        assert!(!valid_email("@no-local.com"));
        assert!(!valid_email("bad@.com"));
        assert!(!valid_email("bad@domain."));
        assert!(!valid_email("bad@nodomain"));
    }

    #[test]
    fn url_validation() {
        assert!(valid_url("https://a.b"));
        assert!(valid_url("http://example.com/path"));
        assert!(!valid_url(""));
        assert!(!valid_url("example.com"));
        assert!(!valid_url("ftp://example.com"));
        assert!(!valid_url("https://"));
    }

    #[test]
    fn length_caps() {
        let ok = FormValues {
            title: "Title".into(),
            description: "Description".into(),
            author: "A".into(),
            email: "a@b.co".into(),
            link: "https://a.b".into(),
        };
        assert!(validate(&ok).is_none());

        let huge_title = FormValues {
            title: "x".repeat(MAX_TITLE + 1),
            ..ok.clone()
        };
        assert_eq!(validate(&huge_title), Some("Title is too long."));

        let huge_desc = FormValues {
            description: "x".repeat(MAX_DESCRIPTION + 1),
            ..ok.clone()
        };
        assert_eq!(validate(&huge_desc), Some("Description is too long."));
    }

    #[test]
    fn form_completeness() {
        let full = FormValues {
            title: "t".into(),
            description: "d".into(),
            author: "a".into(),
            email: "e".into(),
            link: "l".into(),
        };
        assert!(full.is_complete());

        let missing = FormValues {
            title: "t".into(),
            ..Default::default()
        };
        assert!(!missing.is_complete());
    }
}
