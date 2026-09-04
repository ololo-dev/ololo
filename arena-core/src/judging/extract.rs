//! The verdict extractor: one cheap completion that turns a judge's prose
//! into the JSON it already states.
//!
//! Models end on a page of analysis often enough that a verdict parser
//! alone is not a contract. The old answer was a "nudge": re-run the whole
//! judge, tools stripped, with its own prose fed back as a tool result and
//! a request for a JSON shape that — for criteria judges — contradicted the
//! system prompt. A thinking model spent a minute reconciling the two, and
//! after two nudges the run failed and restarted its investigation from
//! scratch (JFV7O5: 5–9 minutes per judge, three attempts, one verdict).
//!
//! This is a different, smaller task: no tools, no evidence pack, no judging.
//! The analysis is the only input and the sheet is the only output. If even
//! that does not parse, the run is a parse error and the queue's retry
//! budget applies — rarely now, since the parser reads fenced and trailing
//! objects on its own.

use std::time::Duration;

use super::{AgentResponse, JudgeError, JudgeLlm};

/// Wall-clock budget for the extractor call: a short prompt and a short
/// answer, on a model that may still think before it writes.
pub const EXTRACT_TIMEOUT: Duration = Duration::from_secs(120);

/// The extractor's system prompt. `keys` is the criteria contract when the
/// judge scores a sheet; `None` asks for the scalar verdict shape.
pub(crate) fn extractor_system(keys: Option<&[String]>) -> String {
    let shape = match keys {
        Some(keys) => {
            let keys_list = keys
                .iter()
                .map(|k| format!("\"{k}\""))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{{\"criteria\": [{{\"key\": \"<key>\", \"score\": <0.0-10.0 or null>, \
                 \"rationale\": \"<why, in the analysis's own words>\", \"evidence\": \
                 [\"probe:<id>\", \"commit:<sha>\", \"file:<path>:<line>\"]}}, ...], \
                 \"feedback\": \"<the analysis's short review>\"}}\n\
                 Include EXACTLY these criteria keys, each once: {keys_list}. Where the \
                 analysis states a score for a key, copy it; where it states none, use \
                 null and say so in that rationale. Copy only evidence the analysis cites."
            )
        }
        None => "{\"rating\": <number>, \"feedback\": \"<text>\"}\n\
                 Copy the rating the analysis states; if it states none, the feedback \
                 must say so and the rating is 0."
            .to_string(),
    };
    format!(
        "You turn a judge's written analysis into the verdict it states. You are not \
         the judge: add nothing, change no score, invent no evidence, and do not \
         re-evaluate the work. Answer with the JSON object only — no prose before or \
         after it, no code fence:\n{shape}"
    )
}

/// The extractor's user turn: the analysis, whole.
pub(crate) fn extractor_user(analysis: &str) -> String {
    format!("The judge's analysis:\n\n{}", analysis.trim())
}

/// One completion: the analysis in, the verdict JSON out. A tool call in
/// reply, a timeout, or a provider error is the caller's parse failure.
pub(crate) async fn extract_verdict(
    judge_llm: &dyn JudgeLlm,
    analysis: &str,
    keys: Option<&[String]>,
) -> Result<String, JudgeError> {
    let system = extractor_system(keys);
    let user = extractor_user(analysis);
    let resp = tokio::time::timeout(
        EXTRACT_TIMEOUT,
        judge_llm.run_agent(&system, &user, Vec::new(), None),
    )
    .await
    .map_err(|_| JudgeError::AiTimeout)??;
    match resp {
        AgentResponse::Final { text } => Ok(text),
        AgentResponse::ToolCall { .. } => Err(JudgeError::AiParseError),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_criteria_extractor_names_the_keys_and_forbids_judging() {
        let keys = vec!["product".to_string(), "tests".to_string()];
        let s = extractor_system(Some(&keys));
        assert!(s.contains("\"product\", \"tests\""), "{s}");
        assert!(s.contains("\"criteria\""), "{s}");
        assert!(s.contains("You are not the judge"), "{s}");
        assert!(!s.contains("\"rating\": <number>"), "{s}");
    }

    #[test]
    fn a_scalar_extractor_asks_for_the_legacy_shape() {
        let s = extractor_system(None);
        assert!(s.contains("\"rating\": <number>"), "{s}");
        assert!(!s.contains("\"criteria\""), "{s}");
    }

    #[test]
    fn the_analysis_travels_whole() {
        let u = extractor_user("  Score: 7. The tests are honest.\n");
        assert_eq!(
            u,
            "The judge's analysis:\n\nScore: 7. The tests are honest."
        );
    }
}
