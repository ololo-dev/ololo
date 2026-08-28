//! Public, unauthenticated endpoints.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::state::AppState;

#[derive(Serialize)]
pub struct TurnstileConfigResponse {
    pub enabled: bool,
    pub sitekey: Option<String>,
}

pub async fn get_turnstile_config(State(state): State<AppState>) -> Response {
    let body = TurnstileConfigResponse {
        enabled: state.turnstile.enabled,
        sitekey: state.turnstile.sitekey.clone(),
    };
    axum::Json(body).into_response()
}

#[derive(Serialize)]
pub struct PublicPlan {
    /// Monthly judge-run limit for the tier.
    pub judge_run_limit: i64,
}

#[derive(Serialize)]
pub struct PublicPlansResponse {
    /// Whether this instance enforces account tiers at all. When false the
    /// limits below are informational: usage is metered, nothing is denied.
    pub enabled: bool,
    pub free: PublicPlan,
    pub premium: PublicPlan,
}

/// `GET /api/public/plans` — the judge-run limits each tier carries.
/// Every field degrades to its default on a DB hiccup: the page must
/// render, just without enforcement claims it cannot verify.
pub async fn get_public_plans(State(state): State<AppState>) -> Response {
    use arena_core::quota;
    let db = &state.db;
    let body = PublicPlansResponse {
        enabled: quota::plans_enabled(db).await.unwrap_or(false),
        free: PublicPlan {
            judge_run_limit: quota::plan_judge_run_limit(db, quota::PLAN_FREE)
                .await
                .unwrap_or(quota::DEFAULT_FREE_JUDGE_RUN_LIMIT),
        },
        premium: PublicPlan {
            judge_run_limit: quota::plan_judge_run_limit(db, quota::PLAN_PREMIUM)
                .await
                .unwrap_or(quota::DEFAULT_PREMIUM_JUDGE_RUN_LIMIT),
        },
    };
    axum::Json(body).into_response()
}

/// One live (lobby or running) session of a public project, as the landing
/// page shows it. The join code doubles as the session-page URL (`/s/<code>`),
/// which is already public via `GET /api/sessions/by-code/:join_code`.
#[derive(Serialize)]
pub struct PublicActiveSession {
    /// Session DB id — the key WS frames (`ProjectSessionUpdate`,
    /// countdown ticks) use, so the landing can match them to rows.
    pub id: uuid::Uuid,
    pub join_code: String,
    pub name: String,
    /// `"lobby"` (open to join) or `"running"` (live).
    pub status: String,
    pub project_name: String,
    pub project_slug: Option<String>,
    pub cover_image_url: Option<String>,
    /// Players currently in the session.
    pub players: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Countdown snapshot at response time: lobby → seconds until autostart,
    /// running → seconds until the session ends (pause windows added back,
    /// mirroring the game-server's `compute_remaining`). The landing ticks it
    /// down client-side. `None` when it cannot be computed (e.g. a running
    /// session with no `started_at`).
    pub seconds_remaining: Option<i64>,
}

#[derive(Serialize)]
pub struct PublicActiveSessionsResponse {
    pub sessions: Vec<PublicActiveSession>,
}

/// Newest lobby/running sessions shown on the landing. Private projects'
/// sessions stay invisible — the join code is an invite, and listing it here
/// publishes it.
const ACTIVE_SESSIONS_LIMIT: usize = 12;

/// `GET /api/public/active-sessions` — unauthenticated. Newest sessions
/// first, regardless of phase.
pub async fn get_active_sessions(State(state): State<AppState>) -> Response {
    use arena_core::entities::{players, projects, sessions};
    use arena_core::session_status::SessionStatus;
    use axum::http::StatusCode;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

    let rows = match sessions::Entity::find()
        .filter(sessions::Column::Status.is_in([SessionStatus::Lobby, SessionStatus::Running]))
        .order_by_desc(sessions::Column::CreatedAt)
        .find_also_related(projects::Entity)
        .all(&state.db)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %e, "public active-sessions query failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let visible: Vec<(sessions::Model, projects::Model)> = rows
        .into_iter()
        .filter_map(|(s, p)| p.filter(|p| p.public).map(|p| (s, p)))
        .collect();

    // One grouped count query instead of a query per session.
    let session_ids: Vec<uuid::Uuid> = visible.iter().map(|(s, _)| s.id).collect();
    let mut player_counts: std::collections::HashMap<uuid::Uuid, i64> =
        std::collections::HashMap::new();
    if !session_ids.is_empty() {
        match players::Entity::find()
            .filter(players::Column::SessionIdFk.is_in(session_ids))
            .all(&state.db)
            .await
        {
            Ok(rows) => {
                for p in rows {
                    *player_counts.entry(p.session_id_fk).or_insert(0) += 1;
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "public active-sessions player count failed");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
    }

    let now = chrono::Utc::now();
    let mut sessions_out: Vec<PublicActiveSession> = visible
        .into_iter()
        .map(|(s, p)| {
            let seconds_remaining = match s.status {
                SessionStatus::Lobby => {
                    // Anchored to created_at, exactly like both lobby timers.
                    let elapsed = now.signed_duration_since(s.created_at).num_seconds().max(0);
                    Some((state.lobby_timer_secs as i64 - elapsed).max(0))
                }
                SessionStatus::Running => s.started_at.map(|started_at| {
                    let elapsed = now.signed_duration_since(started_at).num_seconds();
                    (p.default_session_duration_secs.max(0) - elapsed
                        + s.paused_duration_secs.unwrap_or(0))
                    .max(0)
                }),
                _ => None,
            };
            PublicActiveSession {
                players: player_counts.get(&s.id).copied().unwrap_or(0),
                id: s.id,
                join_code: s.join_code,
                name: s.name,
                status: s.status.to_string(),
                project_name: p.name,
                project_slug: p.slug,
                cover_image_url: p.cover_image_url,
                created_at: s.created_at,
                started_at: s.started_at,
                seconds_remaining,
            }
        })
        .collect();
    // Newest first, regardless of phase — the query already ordered by
    // created_at desc, and the landing keeps inserting WS-announced sessions
    // at the top under the same rule.
    sessions_out.truncate(ACTIVE_SESSIONS_LIMIT);

    axum::Json(PublicActiveSessionsResponse {
        sessions: sessions_out,
    })
    .into_response()
}

#[derive(Serialize)]
pub struct PublicJudge {
    pub slug: String,
    pub name: String,
    pub description: String,
}

/// `GET /api/public/judges` — the review bench the landing shows: every
/// judge whose rating scale awards points, sorted by name. Penalty judges
/// (negative scales — anti-cheat, golf-verify, from-scratch) are fair-play
/// controls, not reviews, and are skipped; a judge whose scale cannot be
/// read is skipped the same way. DB hiccups degrade to an empty list — the
/// landing falls back to its static copy rather than 500ing.
pub async fn get_public_judges(State(state): State<AppState>) -> Response {
    use arena_core::entities::judges;
    use sea_orm::{EntityTrait, QueryOrder};

    let rows = match judges::Entity::find()
        .order_by_asc(judges::Column::Name)
        .all(&state.db)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %e, "public judges query failed");
            Vec::new()
        }
    };
    let judges: Vec<PublicJudge> = rows
        .into_iter()
        .filter(|j| {
            j.rating_scale
                .get("min")
                .and_then(|v| v.as_f64())
                .is_some_and(|min| min >= 0.0)
        })
        .map(|j| PublicJudge {
            slug: j.slug,
            name: j.name,
            description: j.description,
        })
        .collect();
    axum::Json(serde_json::json!({ "judges": judges })).into_response()
}
