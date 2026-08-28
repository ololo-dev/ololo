//! `tests` entity — probe-loop schema.
//!
//! One row per generated test for a `(session, task)` pair.  Stores the shell
//! command template (with optional `{{var}}` placeholders), the answer template
//! for server-side evaluation, and fixture variable definitions as a JSON blob.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "tests")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Shell command template with optional `{{var}}` placeholders.
    pub command_template: String,
    /// Answer template evaluated against sampled fixture values.
    pub answer_template: String,
    /// JSON array of `FixtureDef`-shaped objects, or `{"kind":"js","script":"..."}`.
    pub fixture_definitions: String,
    pub created_at: ChronoDateTimeUtc,
    pub session_id: Uuid,
    pub task_id: Uuid,
    pub ordinal: i32,
    pub prompt: String,
    /// The probe section's prose from the task markdown — the author's own
    /// explanation of what this check verifies. For judge-registered probes,
    /// the judge's instruction. NULL on legacy rows.
    pub description: Option<String>,
    /// Extended probe config (`crate::evaluation::ProbeConfig`): mode,
    /// executor, schedule, opt-in points, report kind. NULL = legacy
    /// participant shell probe, byte-for-byte today's behavior.
    #[sea_orm(column_type = "Json", nullable)]
    pub probe_config: Option<Json>,
    /// Who declared this probe: "system" (task/judge config) or "judge"
    /// (registered live during a judge run).
    pub initiator: String,
    /// The judge that declared or registered this probe, when `initiator`
    /// is not plain task config. No FK constraint (SQLite ALTER limitation).
    pub registered_by_judge_id: Option<Uuid>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::probes::Entity")]
    Probes,
}

impl Related<super::probes::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Probes.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
