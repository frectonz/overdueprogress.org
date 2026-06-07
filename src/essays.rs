use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use minijinja::context;
use serde::Serialize;
use sqlx::SqlitePool;

use crate::AppState;
use crate::error::AppError;

const CATEGORIES: &[(&str, &str)] = &[
    ("money", "Money, Currency & Financial Systems"),
    ("education", "Education & Human Capital"),
    ("agriculture", "Agriculture, Food & Supply Chains"),
    ("health", "Health"),
    ("cities", "Cities, Housing & Transport"),
    ("energy", "Energy, Technology & Digital Life"),
    ("state", "State, Institutions & Governance"),
    ("society", "History, Culture & Society"),
    ("africa", "Africa & Continental Progress"),
    ("personal", "Personal Essays & Reflections"),
];

fn category_of(id: i64) -> Option<&'static str> {
    Some(match id {
        2 | 3 | 11 | 30 | 31 | 61 | 64 | 77 | 89 => "money",
        6 | 12 | 26 | 33 | 42 | 49 | 51 | 52 | 54 | 84 => "education",
        43 | 45 | 47 | 53 | 57 | 59 | 63 | 86 => "agriculture",
        13 | 17 => "health",
        7 | 25 | 41 | 67 | 69 | 78 | 81 | 88 => "cities",
        14 | 15 | 19 | 20 | 36 | 50 | 56 => "energy",
        10 | 21 | 24 | 32 | 65 | 68 | 71 | 72 | 74 | 80 => "state",
        18 | 27 | 28 | 44 | 58 | 60 | 62 | 66 | 75 | 79 | 87 => "society",
        4 | 73 | 90 => "africa",
        5 | 48 | 82 => "personal",
        _ => return None,
    })
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/essays", get(index))
        .route("/essays/{slug}", get(category))
}

#[derive(Serialize, sqlx::FromRow)]
struct Essay {
    id: i64,
    title: String,
    author: String,
    link: String,
}

#[derive(Serialize)]
struct CategoryCount {
    slug: &'static str,
    title: &'static str,
    count: usize,
}

async fn load_essays(db: &SqlitePool) -> Result<Vec<Essay>, AppError> {
    let rows = sqlx::query_as::<_, Essay>(
        "SELECT id, title, author, link FROM submissions ORDER BY id ASC",
    )
    .fetch_all(db)
    .await?;
    Ok(rows)
}

async fn index(State(state): State<AppState>) -> Result<Response, AppError> {
    let essays = load_essays(&state.db).await?;
    let total = essays.len();
    let categories: Vec<CategoryCount> = CATEGORIES
        .iter()
        .map(|&(slug, title)| CategoryCount {
            slug,
            title,
            count: essays
                .iter()
                .filter(|e| category_of(e.id) == Some(slug))
                .count(),
        })
        .collect();

    Ok(state.view.render(
        "essays_index.html",
        context! { page_title => "The Essays", total, categories },
    ))
}

async fn category(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Response, AppError> {
    let Some(&(_, title)) = CATEGORIES.iter().find(|&&(s, _)| s == slug) else {
        return Ok((StatusCode::NOT_FOUND, "Not Found").into_response());
    };

    let essays: Vec<Essay> = load_essays(&state.db)
        .await?
        .into_iter()
        .filter(|e| category_of(e.id) == Some(slug.as_str()))
        .collect();

    Ok(state.view.render(
        "essays_category.html",
        context! { page_title => title, title, essays },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_slugs_are_unique() {
        let mut slugs: Vec<&str> = CATEGORIES.iter().map(|&(s, _)| s).collect();
        let len = slugs.len();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(slugs.len(), len, "duplicate category slug");
    }

    #[test]
    fn category_of_only_returns_known_slugs() {
        for id in 0..200 {
            if let Some(slug) = category_of(id) {
                assert!(
                    CATEGORIES.iter().any(|&(s, _)| s == slug),
                    "id {id} maps to unknown slug {slug}"
                );
            }
        }
    }
}
