//! `POST /api/personal-projects/suggest-tasks` — a draft navigation map for
//! a description, from the `project_ai` model. Nothing is stored: the form
//! shows the draft and the user keeps, edits or discards it.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use arena_core::personal;
use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::jwt::AccessClaims;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuggestReq {
    pub description: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SuggestedTask {
    pub title: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Serialize)]
pub struct SuggestResp {
    pub tasks: Vec<SuggestedTask>,
}

#[derive(Debug, thiserror::Error)]
pub enum SuggestError {
    #[error("forbidden")]
    Forbidden,
    #[error("description_empty")]
    DescriptionEmpty,
    #[error("description_too_long")]
    DescriptionTooLong,
    #[error("rate_limited")]
    RateLimited,
    #[error("no_model_configured")]
    NoModelConfigured,
    #[error("ai_timeout")]
    AiTimeout,
    #[error("ai_error")]
    AiError,
    #[error("ai_parse_error")]
    AiParseError,
}

impl From<crate::llm::LlmError> for SuggestError {
    fn from(e: crate::llm::LlmError) -> Self {
        match e {
            crate::llm::LlmError::Timeout => SuggestError::AiTimeout,
            crate::llm::LlmError::AiError(_) => SuggestError::AiError,
        }
    }
}

crate::api::error::impl_api_error!(SuggestError {
    Self::Forbidden => (FORBIDDEN, "forbidden"),
    Self::DescriptionEmpty => (BAD_REQUEST, "description_empty"),
    Self::DescriptionTooLong => (BAD_REQUEST, "description_too_long"),
    Self::RateLimited => (TOO_MANY_REQUESTS, "rate_limited"),
    Self::NoModelConfigured => (UNPROCESSABLE_ENTITY, "no_model_configured"),
    Self::AiTimeout => (GATEWAY_TIMEOUT, "ai_timeout"),
    Self::AiError => (BAD_GATEWAY, "ai_error"),
    Self::AiParseError => (BAD_GATEWAY, "ai_parse_error"),
});

/// Hardcoded; the user's words go in the user message only.
const SYSTEM: &str = "\
You plan work on an EXISTING software project. Split the task the user describes into a \
navigation map: 2 to 6 steps, in the order a developer would do them.\n\
Rules:\n\
- Every step is one coherent, reviewable change that leaves the code working.\n\
- Do not add setup, research, review or release steps unless the task asks for them.\n\
- A small task may need only 2 steps; never pad the map.\n\
- Each step has exactly two fields:\n\
  - \"title\": imperative, 3-10 words\n\
  - \"description\": 1-2 sentences saying what is true when the step is finished\n\
- No markdown, no code blocks, no commands in the fields.\n\
Return ONLY a valid JSON array — no explanation, no preamble, no code fence.\n\
Example: [{\"title\":\"...\",\"description\":\"...\"}]";

/// Suggestions per user per window: each one is a paid model call.
const MAX_PER_WINDOW: usize = 10;
const WINDOW: Duration = Duration::from_secs(10 * 60);

fn allow(user_id: Uuid, now: Instant) -> bool {
    static CALLS: Mutex<Option<HashMap<Uuid, VecDeque<Instant>>>> = Mutex::new(None);
    let mut guard = CALLS.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    let cutoff = now.checked_sub(WINDOW).unwrap_or(now);
    if map.len() > 10_000 {
        map.retain(|_, calls| calls.back().is_some_and(|t| *t >= cutoff));
    }
    let calls = map.entry(user_id).or_default();
    while calls.front().is_some_and(|t| *t < cutoff) {
        calls.pop_front();
    }
    if calls.len() >= MAX_PER_WINDOW {
        return false;
    }
    calls.push_back(now);
    true
}

/// Parse the model's answer into at most [`personal::MAX_TASKS`] steps,
/// dropping entries with no title and trimming overlong fields.
pub(super) fn parse(raw: &str) -> Result<Vec<SuggestedTask>, SuggestError> {
    let s = raw.trim();
    let body = s
        .strip_prefix("```json")
        .or_else(|| s.strip_prefix("```"))
        .map(|rest| rest.trim_end_matches("```").trim())
        .unwrap_or(s);
    // Some models wrap the array in prose despite the prompt: take the
    // outermost brackets.
    let body = match (body.find('['), body.rfind(']')) {
        (Some(start), Some(end)) if end > start => &body[start..=end],
        _ => body,
    };
    let drafts: Vec<SuggestedTask> =
        serde_json::from_str(body).map_err(|_| SuggestError::AiParseError)?;
    let tasks: Vec<SuggestedTask> = drafts
        .into_iter()
        .filter_map(|t| {
            let title: String = t
                .title
                .trim()
                .chars()
                .take(personal::MAX_TASK_TITLE_CHARS)
                .collect();
            (!title.is_empty()).then(|| SuggestedTask {
                title,
                description: t
                    .description
                    .trim()
                    .chars()
                    .take(personal::MAX_TASK_DESCRIPTION_CHARS)
                    .collect(),
            })
        })
        .take(personal::MAX_TASKS)
        .collect();
    if tasks.is_empty() {
        return Err(SuggestError::AiParseError);
    }
    Ok(tasks)
}

pub async fn suggest_tasks(
    State(state): State<AppState>,
    claims: AccessClaims,
    Json(req): Json<SuggestReq>,
) -> Result<Json<SuggestResp>, SuggestError> {
    let user_id = claims.user_id().map_err(|_| SuggestError::Forbidden)?;
    let description = req.description.trim();
    if description.is_empty() {
        return Err(SuggestError::DescriptionEmpty);
    }
    if description.chars().count() > personal::MAX_DESCRIPTION_CHARS {
        return Err(SuggestError::DescriptionTooLong);
    }
    let candidates = crate::llm::resolve_candidates_for_operation(
        &state.db,
        &state.settings_encryption,
        "project_ai",
    )
    .await;
    if candidates.is_empty() {
        return Err(SuggestError::NoModelConfigured);
    }
    if !allow(user_id, Instant::now()) {
        return Err(SuggestError::RateLimited);
    }
    let name = req
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or("(unnamed)");
    let user = format!("Project: {name}\n\nTask:\n{description}");
    let raw = crate::llm::complete_with_failover(
        &state,
        &candidates,
        "project_ai",
        arena_core::llm::telemetry::LlmContext::default(),
        SYSTEM,
        &user,
        Duration::from_secs(120),
    )
    .await?;
    Ok(Json(SuggestResp {
        tasks: parse(&raw)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_fenced_array_and_drops_empty_titles() {
        let raw = "```json\n[{\"title\":\" Add the endpoint \",\"description\":\"GET /x.csv\"},\
                   {\"title\":\"  \"},{\"title\":\"Add the button\"}]\n```";
        let tasks = parse(raw).expect("parses");
        assert_eq!(
            tasks,
            [
                SuggestedTask {
                    title: "Add the endpoint".into(),
                    description: "GET /x.csv".into()
                },
                SuggestedTask {
                    title: "Add the button".into(),
                    description: String::new()
                },
            ]
        );
    }

    #[test]
    fn digs_the_array_out_of_prose_and_caps_the_map() {
        let many: Vec<String> = (0..15)
            .map(|i| format!("{{\"title\":\"Step {i}\",\"description\":\"d\"}}"))
            .collect();
        let raw = format!("Sure! Here is the plan:\n[{}]\nGood luck.", many.join(","));
        let tasks = parse(&raw).expect("parses");
        assert_eq!(tasks.len(), personal::MAX_TASKS);
        assert_eq!(tasks[0].title, "Step 0");
    }

    #[test]
    fn an_answer_without_steps_is_a_parse_error() {
        assert!(matches!(parse("[]"), Err(SuggestError::AiParseError)));
        assert!(matches!(parse("no idea"), Err(SuggestError::AiParseError)));
    }

    #[test]
    fn the_limiter_admits_a_window_worth_then_refuses() {
        let user = Uuid::new_v4();
        let t0 = Instant::now();
        for _ in 0..MAX_PER_WINDOW {
            assert!(allow(user, t0));
        }
        assert!(!allow(user, t0));
        assert!(allow(Uuid::new_v4(), t0), "other users are not affected");
        assert!(allow(user, t0 + WINDOW + Duration::from_secs(1)));
    }
}
