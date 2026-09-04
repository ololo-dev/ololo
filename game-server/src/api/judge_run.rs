use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use arena_core::judging::{JudgeError, JudgeRunOutput};

use crate::judge_queue;
use crate::state::GameServerState;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgeRunRequest {
    pub session_id: Uuid,
    pub player_id: Uuid,
    pub task_id: Uuid,
    pub judge_id: Uuid,
    /// A human asked for this run by name: overwrite an existing verdict
    /// instead of handing the old one back. Defaults to off so a server that
    /// predates the field still calls the endpoint successfully.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgeRunResponse {
    pub judge_result_id: Uuid,
    pub rating: f64,
    pub feedback: String,
    pub point_delta: i32,
    pub raw_output: String,
    pub model: String,
}

impl From<JudgeRunOutput> for JudgeRunResponse {
    fn from(o: JudgeRunOutput) -> Self {
        Self {
            judge_result_id: o.judge_result_id,
            rating: o.rating,
            feedback: o.feedback,
            point_delta: o.point_delta,
            raw_output: o.raw_output,
            model: o.model,
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum JudgeRunApiError {
    #[error(transparent)]
    Judge(#[from] JudgeError),
}

impl IntoResponse for JudgeRunApiError {
    fn into_response(self) -> Response {
        let (status, label) = match &self {
            JudgeRunApiError::Judge(JudgeError::AiTimeout) => {
                (StatusCode::GATEWAY_TIMEOUT, "ai_timeout")
            }
            JudgeRunApiError::Judge(JudgeError::AiParseError) => {
                (StatusCode::BAD_GATEWAY, "ai_parse_error")
            }
            JudgeRunApiError::Judge(JudgeError::AiRatingOutOfRange) => {
                (StatusCode::BAD_GATEWAY, "ai_rating_out_of_range")
            }
            JudgeRunApiError::Judge(JudgeError::FeedbackTooLong) => {
                (StatusCode::BAD_GATEWAY, "feedback_too_long")
            }
            JudgeRunApiError::Judge(JudgeError::TooManyToolCalls) => {
                (StatusCode::BAD_GATEWAY, "too_many_tool_calls")
            }
            // The queue converts a pause into a `waiting` row before it
            // reaches an API caller; kept for exhaustiveness.
            JudgeRunApiError::Judge(JudgeError::Suspended(_)) => {
                (StatusCode::ACCEPTED, "waiting_for_participant")
            }
            JudgeRunApiError::Judge(JudgeError::Db(_)) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "database_error")
            }
            JudgeRunApiError::Judge(JudgeError::Llm(_)) => (StatusCode::BAD_GATEWAY, "llm_error"),
            JudgeRunApiError::Judge(JudgeError::QuotaExceeded(_)) => {
                (StatusCode::TOO_MANY_REQUESTS, "judge_quota_exceeded")
            }
            JudgeRunApiError::Judge(JudgeError::GitReadError(msg)) => {
                if msg.contains("not found") {
                    (StatusCode::NOT_FOUND, "not_found")
                } else {
                    (StatusCode::BAD_GATEWAY, "git_read_error")
                }
            }
            JudgeRunApiError::Judge(JudgeError::PlayerRepoNotFound) => {
                (StatusCode::NOT_FOUND, "player_repo_not_found")
            }
            JudgeRunApiError::Judge(JudgeError::TaskNotFound) => {
                (StatusCode::NOT_FOUND, "task_not_found")
            }
            JudgeRunApiError::Judge(JudgeError::PlayerRepoEmpty) => {
                (StatusCode::BAD_GATEWAY, "player_repo_empty")
            }
            JudgeRunApiError::Judge(JudgeError::ExecTimeout) => {
                (StatusCode::GATEWAY_TIMEOUT, "exec_timeout")
            }
            JudgeRunApiError::Judge(JudgeError::ExecFailed(_)) => {
                (StatusCode::BAD_GATEWAY, "exec_failed")
            }
            // The judge's own program is broken or returned nothing usable —
            // an authoring fault in the definition, not a fault of the run.
            JudgeRunApiError::Judge(JudgeError::DecideFailed(_)) => {
                (StatusCode::UNPROCESSABLE_ENTITY, "decide_failed")
            }
            // The model answered and the judge's review threw the answer out.
            // Retryable, so by the time it surfaces here every attempt has
            // been spent.
            JudgeRunApiError::Judge(JudgeError::VerdictRejected(_)) => {
                (StatusCode::BAD_GATEWAY, "verdict_rejected")
            }
        };
        let JudgeRunApiError::Judge(inner) = &self;
        let body = Json(serde_json::json!({ "error": label, "detail": inner.to_string() }));
        (status, body).into_response()
    }
}

pub async fn post_run(
    State(state): State<GameServerState>,
    Json(req): Json<JudgeRunRequest>,
) -> Result<Json<JudgeRunResponse>, JudgeRunApiError> {
    // The run outlives the request. hyper drops a handler's future when the
    // client goes away, and the client here is the main server relaying an
    // admin's browser call through Cloudflare, which hangs up at 100 s — a
    // session report takes longer than that, so the re-run died unwritten
    // (4I2GFR). A detached task keeps running; the response still waits
    // for it when the connection lasts.
    let task = tokio::spawn(async move {
        judge_queue::enqueue_judge_run(
            &state,
            &state.db,
            req.session_id,
            req.player_id,
            req.task_id,
            req.judge_id,
            req.force,
        )
        .await
    });
    let out = task
        .await
        .map_err(|e| JudgeRunApiError::Judge(JudgeError::Llm(format!("judge run task: {e}"))))??;
    Ok(Json(JudgeRunResponse::from(out)))
}
