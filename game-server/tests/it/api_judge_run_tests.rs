//! Coverage for src/api/judge_run.rs: response mapping, error→status mapping,
//! and request (de)serialization.

use arena_core::judging::{JudgeError, JudgeRunOutput};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use game_server::api::judge_run::{JudgeRunApiError, JudgeRunRequest, JudgeRunResponse};
use uuid::Uuid;

fn sample_output() -> JudgeRunOutput {
    arena_core::judging::JudgeRunOutput {
        rating: 8.5,
        point_delta: 12,
        feedback: "solid".to_string(),
        raw_output: "raw".to_string(),
        model: "llama3.2".to_string(),
        judge_result_id: Uuid::new_v4(),
        duration_ms: 1500,
    }
}

#[tokio::test]
async fn response_from_output_maps_all_fields() {
    let out = sample_output();
    let resp: JudgeRunResponse = out.clone().into();
    assert_eq!(resp.judge_result_id, out.judge_result_id);
    assert_eq!(resp.rating, out.rating);
    assert_eq!(resp.feedback, out.feedback);
    assert_eq!(resp.point_delta, out.point_delta);
    assert_eq!(resp.raw_output, out.raw_output);
    assert_eq!(resp.model, out.model);
}

async fn error_label(err: JudgeError) -> (StatusCode, String) {
    let resp = JudgeRunApiError::from(err).into_response();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let value: serde_json::Value = serde_json::from_slice(&body).expect("json");
    (status, value["error"].as_str().unwrap().to_string())
}

#[tokio::test]
async fn ai_timeout_maps_gateway_timeout() {
    let (s, l) = error_label(arena_core::judging::JudgeError::AiTimeout).await;
    assert_eq!(s, StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(l, "ai_timeout");
}

#[tokio::test]
async fn ai_parse_error_maps_bad_gateway() {
    let (s, l) = error_label(arena_core::judging::JudgeError::AiParseError).await;
    assert_eq!(s, StatusCode::BAD_GATEWAY);
    assert_eq!(l, "ai_parse_error");
}

#[tokio::test]
async fn ai_rating_out_of_range_maps_bad_gateway() {
    let (s, l) = error_label(arena_core::judging::JudgeError::AiRatingOutOfRange).await;
    assert_eq!(s, StatusCode::BAD_GATEWAY);
    assert_eq!(l, "ai_rating_out_of_range");
}

#[tokio::test]
async fn feedback_too_long_maps_bad_gateway() {
    let (s, l) = error_label(arena_core::judging::JudgeError::FeedbackTooLong).await;
    assert_eq!(s, StatusCode::BAD_GATEWAY);
    assert_eq!(l, "feedback_too_long");
}

#[tokio::test]
async fn too_many_tool_calls_maps_bad_gateway() {
    let (s, l) = error_label(arena_core::judging::JudgeError::TooManyToolCalls).await;
    assert_eq!(s, StatusCode::BAD_GATEWAY);
    assert_eq!(l, "too_many_tool_calls");
}

#[tokio::test]
async fn db_error_maps_internal_server_error() {
    let db_err = sea_orm::DbErr::Custom("boom".to_string());
    let (s, l) = error_label(arena_core::judging::JudgeError::Db(db_err)).await;
    assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(l, "database_error");
}

#[tokio::test]
async fn git_read_not_found_maps_not_found() {
    let (s, l) = error_label(arena_core::judging::JudgeError::GitReadError(
        "task_judge not found".to_string(),
    ))
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(l, "not_found");
}

#[tokio::test]
async fn git_read_other_maps_bad_gateway() {
    let (s, l) = error_label(arena_core::judging::JudgeError::GitReadError(
        "weird".to_string(),
    ))
    .await;
    assert_eq!(s, StatusCode::BAD_GATEWAY);
    assert_eq!(l, "git_read_error");
}

#[tokio::test]
async fn player_repo_not_found_maps_not_found() {
    let (s, l) = error_label(arena_core::judging::JudgeError::PlayerRepoNotFound).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(l, "player_repo_not_found");
}

#[tokio::test]
async fn player_repo_empty_maps_bad_gateway() {
    let (s, l) = error_label(arena_core::judging::JudgeError::PlayerRepoEmpty).await;
    assert_eq!(s, StatusCode::BAD_GATEWAY);
    assert_eq!(l, "player_repo_empty");
}

#[tokio::test]
async fn request_parses_known_fields() {
    let sid = Uuid::new_v4();
    let v = serde_json::json!({
        "session_id": sid,
        "player_id": Uuid::new_v4(),
        "task_id": Uuid::new_v4(),
        "judge_id": Uuid::new_v4(),
    });
    let req: JudgeRunRequest = serde_json::from_value(v).expect("parse");
    assert_eq!(req.session_id, sid);
    // A caller that predates `force` gets the safe reading: this is not a
    // hand-ordered re-run, so the cost guard stays armed.
    assert!(!req.force);
}

#[tokio::test]
async fn force_is_carried_when_asked_for() {
    let v = serde_json::json!({
        "session_id": Uuid::new_v4(),
        "player_id": Uuid::new_v4(),
        "task_id": Uuid::new_v4(),
        "judge_id": Uuid::new_v4(),
        "force": true,
    });
    let req: JudgeRunRequest = serde_json::from_value(v).expect("parse");
    assert!(req.force);
}

#[tokio::test]
async fn request_rejects_unknown_fields() {
    let v = serde_json::json!({
        "session_id": Uuid::new_v4(),
        "player_id": Uuid::new_v4(),
        "task_id": Uuid::new_v4(),
        "judge_id": Uuid::new_v4(),
        "extra": "no",
    });
    assert!(serde_json::from_value::<JudgeRunRequest>(v).is_err());
}
