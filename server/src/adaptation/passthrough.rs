//! Passthrough adaptation service — no LLM required.
//!
//! Parses fenced code blocks from `command_template` and inserts one
//! `tests` row per block. If no blocks are found (plain command), inserts
//! a single `tests` row from the command directly.
//!
//! Idempotent: if any `tests` rows already exist for
//! `(session_id, task_id)`, returns `Ok(())` immediately.
use crate::adaptation::command_policy::split_when_safe;
use crate::adaptation::service::{AdaptationError, AdaptationRequest, AdaptationService};
use arena_core::entities::tests as entity_tests;
use arena_core::task_template::parse_structured_markdown_tests;
use async_trait::async_trait;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set,
    TransactionTrait,
};
use uuid::Uuid;

pub struct PassthroughAdaptationService {
    db: DatabaseConnection,
}

impl PassthroughAdaptationService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}

/// Parse fenced code blocks (triple-backtick) from markdown text.
/// Returns the trimmed content of each block.
pub(crate) fn parse_code_blocks(md: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<Vec<&str>> = None;

    for line in md.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            match current.take() {
                None => {
                    // Opening fence
                    current = Some(Vec::new());
                }
                Some(lines) => {
                    // Closing fence — only when the line is exactly ``` (after trim)
                    if trimmed == "```" {
                        blocks.push(lines.join("\n").trim().to_owned());
                    } else {
                        // Another opening fence inside? Treat previous as unclosed, start new.
                        current = Some(Vec::new());
                    }
                }
            }
        } else if let Some(ref mut lines) = current {
            lines.push(line);
        }
    }

    blocks
}

#[async_trait]
impl AdaptationService for PassthroughAdaptationService {
    async fn adapt(&self, req: AdaptationRequest) -> Result<(), AdaptationError> {
        let session_id = req.session_id;
        let task_id = req.task_id;

        // Idempotent: skip if rows already exist for this (session, task).
        let existing = entity_tests::Entity::find()
            .filter(entity_tests::Column::SessionId.eq(session_id))
            .filter(entity_tests::Column::TaskId.eq(task_id))
            .one(&self.db)
            .await
            .map_err(|e| AdaptationError::DbError(e.to_string()))?;
        if existing.is_some() {
            return Ok(());
        }

        let structured = parse_structured_markdown_tests(&req.command_template);
        let tx = self
            .db
            .begin()
            .await
            .map_err(|e| AdaptationError::DbError(e.to_string()))?;

        if !structured.is_empty() {
            for (ordinal, t) in structured.iter().enumerate() {
                // Seed validation rejects malformed fences; a failure here is
                // a hand-edited template — safest reading is "legacy probe".
                let probe_config = t.parsed_probe_config().unwrap_or_else(|e| {
                    tracing::warn!(%task_id, ordinal, error = %e,
                        "yaml probe fence unparseable; treating section as legacy probe");
                    None
                });
                entity_tests::ActiveModel {
                    id: Set(Uuid::new_v4()),
                    command_template: Set(t.command_template.clone()),
                    answer_template: Set(t.answer_template.clone()),
                    fixture_definitions: Set(t.fixture_definitions.clone()),
                    created_at: Set(Utc::now()),
                    session_id: Set(session_id),
                    task_id: Set(task_id),
                    ordinal: Set(ordinal as i32),
                    // The test's `## ` heading — the human label the UI shows
                    // for probes of this test. Legacy placeholder when absent.
                    prompt: Set(if t.title.trim().is_empty() {
                        format!("Structured markdown test {ordinal}")
                    } else {
                        t.title.trim().to_string()
                    }),
                    description: Set(Some(t.description.clone()).filter(|s| !s.is_empty())),
                    probe_config: Set(probe_config
                        .as_ref()
                        .and_then(|c| serde_json::to_value(c).ok())),
                    initiator: Set(arena_core::evaluation::INITIATOR_SYSTEM.to_string()),
                    registered_by_judge_id: Set(None),
                }
                .insert(&tx)
                .await
                .map_err(|e| AdaptationError::DbError(e.to_string()))?;
            }
            tx.commit()
                .await
                .map_err(|e| AdaptationError::DbError(e.to_string()))?;
            return Ok(());
        }

        let blocks = parse_code_blocks(&req.command_template);
        if blocks.is_empty() {
            // Plain command — insert one tests row directly.
            if req.command_template.trim().is_empty() {
                return Err(AdaptationError::EmptyCommandTemplate);
            }
            entity_tests::ActiveModel {
                id: Set(Uuid::new_v4()),
                command_template: Set(req.command_template.clone()),
                answer_template: Set(req.answer_template.clone()),
                fixture_definitions: Set(req.fixture_definitions.clone()),
                created_at: Set(Utc::now()),
                session_id: Set(session_id),
                task_id: Set(task_id),
                ordinal: Set(0),
                prompt: Set(String::new()),
                description: Set(None),
                probe_config: Set(None),
                initiator: Set(arena_core::evaluation::INITIATOR_SYSTEM.to_string()),
                registered_by_judge_id: Set(None),
            }
            .insert(&tx)
            .await
            .map_err(|e| AdaptationError::DbError(e.to_string()))?;
            return tx
                .commit()
                .await
                .map_err(|e| AdaptationError::DbError(e.to_string()));
        }

        {
            let mut ordinal = 0i32;
            for block in &blocks {
                let commands = split_when_safe(block.trim())
                    .map_err(|e| AdaptationError::LlmFailed(e.as_code().to_string()))?;
                for command in commands {
                    if command.trim().is_empty() {
                        return Err(AdaptationError::EmptyCommandTemplate);
                    }
                    entity_tests::ActiveModel {
                        id: Set(Uuid::new_v4()),
                        command_template: Set(command.clone()),
                        answer_template: Set(req.answer_template.clone()),
                        fixture_definitions: Set(req.fixture_definitions.clone()),
                        created_at: Set(Utc::now()),
                        session_id: Set(session_id),
                        task_id: Set(task_id),
                        ordinal: Set(ordinal),
                        prompt: Set(format!("Passthrough test {ordinal}")),
                        description: Set(None),
                        probe_config: Set(None),
                        initiator: Set(arena_core::evaluation::INITIATOR_SYSTEM.to_string()),
                        registered_by_judge_id: Set(None),
                    }
                    .insert(&tx)
                    .await
                    .map_err(|e| AdaptationError::DbError(e.to_string()))?;
                    ordinal += 1;
                }
            }
        }

        tx.commit()
            .await
            .map_err(|e| AdaptationError::DbError(e.to_string()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::parse_code_blocks;

    #[test]
    fn test_no_blocks() {
        let md = "No code here, just text.";
        assert!(parse_code_blocks(md).is_empty());
    }

    #[test]
    fn test_single_block_no_lang() {
        let md = "```\necho hello\n```";
        let blocks = parse_code_blocks(md);
        assert_eq!(blocks, vec!["echo hello"]);
    }

    #[test]
    fn test_single_block_with_lang() {
        let md = "```bash\necho hello\n```";
        let blocks = parse_code_blocks(md);
        assert_eq!(blocks, vec!["echo hello"]);
    }

    #[test]
    fn test_multiple_blocks() {
        let md = "```\nfirst\n```\n\nsome text\n\n```rust\nsecond\n```";
        let blocks = parse_code_blocks(md);
        assert_eq!(blocks, vec!["first", "second"]);
    }

    #[test]
    fn test_block_whitespace_trimmed() {
        let md = "```\n\n  trimmed  \n\n```";
        let blocks = parse_code_blocks(md);
        assert_eq!(blocks, vec!["trimmed"]);
    }
}
