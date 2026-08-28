//! Admin email template management API.
//!
//! Routes (mounted by `lib.rs`):
//!   GET /api/admin/email-templates       — list all email templates (admin only)
//!   PUT /api/admin/email-templates/:type — update a template (admin only)

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use serde::{Deserialize, Serialize};

use crate::api::error::error_response;
use crate::api::settings::AdminUser;
use crate::state::AppState;
use arena_core::entities::email_templates;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum EmailTemplateError {
    #[error("database error")]
    Db(#[from] sea_orm::DbErr),
    #[error("invalid template type")]
    InvalidType,
    #[error("missing required placeholder: {0}")]
    MissingPlaceholder(String),
}

impl IntoResponse for EmailTemplateError {
    fn into_response(self) -> Response {
        match self {
            Self::Db(e) => {
                tracing::error!("email template DB error: {e}");
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
            }
            Self::InvalidType => error_response(StatusCode::BAD_REQUEST, "invalid_template_type"),
            Self::MissingPlaceholder(ph) => error_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                &format!("template missing required placeholder {}", ph),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct EmailTemplateDto {
    pub r#type: String,
    pub subject: String,
    pub body_html: String,
    pub body_text: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize)]
pub struct UpdateEmailTemplateReq {
    pub subject: String,
    pub body_html: String,
    pub body_text: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Placeholders the given template type cannot work without, or `None` if the
/// type is unknown. A template saved without one of these would break only
/// at send time, for the recipient — not something to discover in an inbox.
fn required_placeholders(template_type: &str) -> Option<&'static [&'static str]> {
    match template_type {
        "verify" => Some(&["{{VERIFY_URL}}"]),
        "reset_password" => Some(&["{{RESET_URL}}"]),
        "magic_link" => Some(&["{{MAGIC_LINK_URL}}"]),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/admin/email-templates` — returns all email templates.
pub async fn get_email_templates(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<EmailTemplateDto>>, EmailTemplateError> {
    let rows = email_templates::Entity::find().all(&state.db).await?;

    let dtos = rows
        .into_iter()
        .map(|m| EmailTemplateDto {
            r#type: m.r#type,
            subject: m.subject,
            body_html: m.body_html,
            body_text: m.body_text,
            updated_at: m.updated_at,
        })
        .collect();

    Ok(Json(dtos))
}

/// `PUT /api/admin/email-templates/:type` — update a single email template.
pub async fn put_email_template(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(template_type): Path<String>,
    Json(body): Json<UpdateEmailTemplateReq>,
) -> Result<Json<serde_json::Value>, EmailTemplateError> {
    let placeholders =
        required_placeholders(&template_type).ok_or(EmailTemplateError::InvalidType)?;

    // Both bodies must carry every one of them: a placeholder present in the
    // HTML and missing from the text part fails only for the recipients whose
    // client shows plain text, which is the hardest kind of breakage to see.
    if let Some(missing) = placeholders
        .iter()
        .find(|p| !body.body_html.contains(**p) || !body.body_text.contains(**p))
    {
        return Err(EmailTemplateError::MissingPlaceholder(
            (*missing).to_string(),
        ));
    }

    // Find the existing row and update it.
    let existing = email_templates::Entity::find_by_id(template_type.clone())
        .one(&state.db)
        .await?
        .ok_or(EmailTemplateError::InvalidType)?;

    let mut active: email_templates::ActiveModel = existing.into();
    active.subject = Set(body.subject);
    active.body_html = Set(body.body_html);
    active.body_text = Set(body.body_text);
    active.updated_at = Set(Utc::now());
    active.update(&state.db).await?;

    Ok(Json(serde_json::json!({ "ok": true })))
}
