use axum::{Router, extract::State, response::Response, routing::get};
use minijinja::context;

use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(index))
        .route("/competition", get(competition))
        .route("/winners", get(winners))
}

async fn index(State(state): State<AppState>) -> Response {
    state.view.render("index.html", context! {})
}

async fn competition(State(state): State<AppState>) -> Response {
    state.view.render(
        "competition.html",
        context! { page_title => "Essay Competition" },
    )
}

async fn winners(State(state): State<AppState>) -> Response {
    state
        .view
        .render("winners.html", context! { page_title => "The Winners" })
}
