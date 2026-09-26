//! `POST /api/personal-projects/suggest-tasks` — a draft navigation map for
//! a description, and a name for the work, from the `project_ai` model.
//! Nothing is stored: the form (or the landing's popup) shows the draft and
//! the user keeps, edits or discards it.

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
    /// A name for the work, when the model gave one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub tasks: Vec<SuggestedTask>,
}

/// The shape the prompt asks for.
#[derive(Debug, Deserialize)]
struct Draft {
    #[serde(default)]
    name: Option<String>,
    #[serde(alias = "steps")]
    tasks: Vec<SuggestedTask>,
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
You plan work on an EXISTING software project. Name the work the user describes and split it \
into a navigation map: 2 to 6 steps, in the order a developer would do them.\n\
Rules:\n\
- \"name\": what the work is, as a title of 2-6 words\n\
- Every step is one coherent, reviewable change that leaves the code working.\n\
- Do not add setup, research, review or release steps unless the task asks for them.\n\
- A small task may need only 2 steps; never pad the map.\n\
- Each step has exactly two fields:\n\
  - \"title\": imperative, 3-10 words\n\
  - \"description\": 1-2 sentences saying what is true when the step is finished\n\
- No markdown, no code blocks, no commands in the fields.\n\
Return ONLY a valid JSON object — no explanation, no preamble, no code fence.\n\
Example: {\"name\":\"...\",\"tasks\":[{\"title\":\"...\",\"description\":\"...\"}]}";

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

/// The text between the first `open` and the last `close`: some models wrap
/// the JSON in prose despite the prompt.
fn outermost(body: &str, open: char, close: char) -> Option<&str> {
    match (body.find(open), body.rfind(close)) {
        (Some(start), Some(end)) if end > start => Some(&body[start..=end]),
        _ => None,
    }
}

/// Parse the model's answer into a name, when it gave a usable one, and at
/// most [`personal::MAX_TASKS`] steps, dropping entries with no title and
/// trimming overlong fields. A bare array of steps — the shape the prompt
/// asked for before it named the work — still reads, without a name.
pub(super) fn parse(raw: &str) -> Result<(Option<String>, Vec<SuggestedTask>), SuggestError> {
    let s = raw.trim();
    let body = s
        .strip_prefix("```json")
        .or_else(|| s.strip_prefix("```"))
        .map(|rest| rest.trim_end_matches("```").trim())
        .unwrap_or(s);
    let (name, drafts) = match outermost(body, '{', '}')
        .and_then(|object| serde_json::from_str::<Draft>(object).ok())
    {
        Some(draft) => (draft.name, draft.tasks),
        None => {
            let array = outermost(body, '[', ']').unwrap_or(body);
            let tasks: Vec<SuggestedTask> =
                serde_json::from_str(array).map_err(|_| SuggestError::AiParseError)?;
            (None, tasks)
        }
    };
    let name = name
        .map(|n| {
            n.trim()
                .trim_matches('"')
                .trim_end_matches('.')
                .trim()
                .chars()
                .take(personal::MAX_NAME_CHARS)
                .collect::<String>()
        })
        .filter(|n| !n.is_empty());
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
    Ok((name, tasks))
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
    let (name, tasks) = parse(&raw)?;
    Ok(Json(SuggestResp { name, tasks }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_fenced_array_and_drops_empty_titles() {
        let raw = "```json\n[{\"title\":\" Add the endpoint \",\"description\":\"GET /x.csv\"},\
                   {\"title\":\"  \"},{\"title\":\"Add the button\"}]\n```";
        let (name, tasks) = parse(raw).expect("parses");
        assert_eq!(name, None, "a bare array names nothing");
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
        let (_, tasks) = parse(&raw).expect("parses");
        assert_eq!(tasks.len(), personal::MAX_TASKS);
        assert_eq!(tasks[0].title, "Step 0");
    }

    #[test]
    fn the_object_answer_names_the_work() {
        let raw = "Here you go: {\"name\": \" \\\"CSV export for reports.\\\" \", \
                   \"tasks\": [{\"title\":\"Add the endpoint\",\"description\":\"GET /x.csv\"}]}";
        let (name, tasks) = parse(raw).expect("parses");
        assert_eq!(name.as_deref(), Some("CSV export for reports"));
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "Add the endpoint");

        let long = format!(
            "{{\"name\":\"{}\",\"steps\":[{{\"title\":\"Only step\"}}]}}",
            "n".repeat(personal::MAX_NAME_CHARS + 20)
        );
        let (name, tasks) = parse(&long).expect("steps is read as tasks");
        assert_eq!(
            name.map(|n| n.chars().count()),
            Some(personal::MAX_NAME_CHARS)
        );
        assert_eq!(tasks[0].title, "Only step");

        let (name, _) = parse("{\"name\":\"  \",\"tasks\":[{\"title\":\"A\"}]}").expect("parses");
        assert_eq!(name, None, "a blank name is no name");
    }

    #[test]
    fn an_answer_without_steps_is_a_parse_error() {
        assert!(matches!(parse("[]"), Err(SuggestError::AiParseError)));
        assert!(matches!(parse("no idea"), Err(SuggestError::AiParseError)));
        assert!(matches!(
            parse("{\"name\":\"X\",\"tasks\":[]}"),
            Err(SuggestError::AiParseError)
        ));
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
