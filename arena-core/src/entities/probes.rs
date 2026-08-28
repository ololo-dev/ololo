//! `probes` entity.
//!
//! One row per rendered probe dispatched to a player.  `outcome` is `None`
//! while the probe is in-flight; set to "pass", "error", or "no_response"
//! once resolved.  Scoring logic applies `point_delta` when `outcome` is set.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "probes")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub test_id: Uuid,
    pub player_id: Uuid,
    pub session_id: Uuid,
    pub attempt: i32,
    pub rendered_command: String,
    /// JSON object: variable-name → sampled string value.
    pub fixture_values: String,
    pub expected_answer: Option<String>,
    /// When the validation script calls `assertEqual(actual, expected)`,
    /// the computed expected value (e.g. "42"). NULL for predicate templates
    /// or pre-evaluated expected_answer equality. Displayed in the
    /// "Expected:" panel; `expected_answer` holds the raw template for the
    /// "Task answer:" panel.
    pub resolved_answer: Option<String>,
    /// JSON `{ "fixtures": ["key", ...], "expected": bool }` metadata
    /// describing which fixture values and the expected-answer value are
    /// secret (must be redacted from clients). NULL when nothing is secret.
    pub secret_meta: Option<String>,
    /// "pass" | "error" | "no_response", or NULL while in-flight.
    pub outcome: Option<String>,
    pub dispatched_at: ChronoDateTimeUtc,
    pub deadline_at: ChronoDateTimeUtc,
    pub resolved_at: Option<ChronoDateTimeUtc>,
    pub updated_at: Option<ChronoDateTimeUtc>,
    pub output: Option<String>,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<i64>,
    pub point_delta: Option<i32>,
    /// Structured measurement: analysis metrics, llm rubric scores, parsed
    /// TODO report, artifact metadata, snapshot commit/age for server-side
    /// runs. NULL for legacy probes.
    #[sea_orm(column_type = "Json", nullable)]
    pub result_json: Option<Json>,
    /// Interactive-probe artifact reference into the player repo, as
    /// `"<commit_sha>:<repo_path>"`. NULL when no artifact.
    pub artifact_path: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::tests::Entity",
        from = "Column::TestId",
        to = "super::tests::Column::Id",
        on_delete = "Cascade"
    )]
    Test,
    #[sea_orm(
        belongs_to = "super::players::Entity",
        from = "Column::PlayerId",
        to = "super::players::Column::Id",
        on_delete = "Cascade"
    )]
    Player,
    #[sea_orm(
        belongs_to = "super::sessions::Entity",
        from = "Column::SessionId",
        to = "super::sessions::Column::Id",
        on_delete = "Cascade"
    )]
    Session,
}

impl Related<super::tests::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Test.def()
    }
}

impl Related<super::players::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Player.def()
    }
}

impl Related<super::sessions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Session.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
