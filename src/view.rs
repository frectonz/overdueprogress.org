use std::sync::Arc;

use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use minijinja::{Environment as Jinja, Value};
use rust_embed::Embed;
use thiserror::Error;

#[derive(Embed, Clone)]
#[folder = "static/"]
pub struct StaticAssets;

#[derive(Embed)]
#[folder = "templates/"]
struct Templates;

#[derive(Clone)]
pub struct View {
    jinja: Arc<Jinja<'static>>,
}

impl View {
    pub fn new() -> Self {
        let mut jinja = Jinja::new();
        jinja.set_loader(|name| {
            Ok(Templates::get(name)
                .and_then(|f| std::str::from_utf8(f.data.as_ref()).ok().map(String::from)))
        });
        Self {
            jinja: Arc::new(jinja),
        }
    }

    pub fn render(&self, name: &str, ctx: Value) -> Response {
        match self.render_to_string(name, ctx) {
            Ok(body) => Html(body).into_response(),
            Err(_) => internal_error(),
        }
    }

    pub fn render_to_string(&self, name: &str, ctx: Value) -> Result<String, ViewError> {
        let tmpl = self.jinja.get_template(name).map_err(|err| {
            tracing::error!(?err, template = name, "template missing");
            ViewError::Missing(name.to_string())
        })?;
        tmpl.render(ctx).map_err(|err| {
            tracing::error!(?err, template = name, "template render failed");
            ViewError::Render(err)
        })
    }
}

#[derive(Debug, Error)]
pub enum ViewError {
    #[error("template not found: {0}")]
    Missing(String),
    #[error("template render failed: {0}")]
    Render(#[from] minijinja::Error),
}

fn internal_error() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error").into_response()
}
