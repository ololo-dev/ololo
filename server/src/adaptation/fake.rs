//! Fake adaptation service for integration tests.
//!
//! Performs real DB writes so that polling loops can find adapted
//! rows. Supports transient-failure injection via `with_failures(n)`.
//! Idempotent: if adapted_tests rows for (session_id, task_id) already
//! exist, returns `Ok(())`.
use crate::adaptation::service::{AdaptationError, AdaptationRequest, AdaptationService};
use arena_core::entities::tests;
use async_trait::async_trait;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    Set,
};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

pub struct FakeAdaptationService {
    db: DatabaseConnection,
    failures_remaining: Arc<Mutex<u32>>,
}

impl FakeAdaptationService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self {
            db,
            failures_remaining: Arc::new(Mutex::new(0)),
        }
    }

    pub fn with_failures(self, n: u32) -> Self {
        *self.failures_remaining.lock().expect("lock") = n;
        self
    }
}

#[async_trait]
impl AdaptationService for FakeAdaptationService {
    async fn adapt(&self, req: AdaptationRequest) -> Result<(), AdaptationError> {
        // Idempotency: if any adapted_tests rows exist for (session_id, task_id),
        // this adaptation already ran — return early.
        let existing: u64 = tests::Entity::find()
            .filter(tests::Column::SessionId.eq(req.session_id))
            .filter(tests::Column::TaskId.eq(req.task_id))
            .count(&self.db)
            .await
            .map_err(|e: sea_orm::DbErr| AdaptationError::DbError(e.to_string()))?;
        if existing > 0 {
            return Ok(());
        }

        {
            let mut guard = self.failures_remaining.lock().expect("lock");
            if *guard > 0 {
                *guard -= 1;
                return Err(AdaptationError::LlmFailed(
                    "fake_injected_failure".to_string(),
                ));
            }
        }

        for ordinal in 0i32..2 {
            tests::ActiveModel {
                id: Set(Uuid::new_v4()),
                command_template: Set(format!("echo 'fake_answer_{ordinal}'")),
                answer_template: Set(format!("fake_answer_{ordinal}")),
                fixture_definitions: Set("[]".to_string()),
                created_at: Set(Utc::now()),
                session_id: Set(req.session_id),
                task_id: Set(req.task_id),
                ordinal: Set(ordinal),
                prompt: Set(format!("fake prompt {ordinal}")),
                description: Set(None),
                probe_config: Set(None),
                initiator: Set(arena_core::evaluation::INITIATOR_SYSTEM.to_string()),
                registered_by_judge_id: Set(None),
            }
            .insert(&self.db)
            .await
            .map_err(|e| AdaptationError::DbError(e.to_string()))?;
        }

        Ok(())
    }
}
