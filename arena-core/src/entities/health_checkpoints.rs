//! `health_checkpoints` entity — one row per snapshot commit the game
//! scores for code health: the client's report (advisory) and the server's
//! own verification of the same commit (authoritative) side by side. See
//! `arena_core::protocol::health` for the string values of `kind`,
//! `client_status` and `server_status`, and for the view the browser gets.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "health_checkpoints")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub session_id_fk: Uuid,
    pub player_id_fk: Uuid,
    /// The task the probe belonged to (or the task the `feat` closed).
    pub task_id_fk: Option<Uuid>,
    /// The probe this commit was made for; NULL for a task-final commit.
    pub probe_id_fk: Option<Uuid>,
    /// `probe` | `task_final`.
    pub kind: String,
    pub probe_seq: i32,
    pub commit_sha: String,
    /// The task the git history attributes the commit to.
    pub derived_task_id: Option<Uuid>,
    pub score_mismatch: bool,
    pub version_mismatch: bool,
    pub task_mismatch: bool,
    pub late: bool,
    pub history_rewritten: bool,
    /// `ok` | `failed` | `timeout` | `skipped`; NULL until a report arrives.
    pub client_status: Option<String>,
    pub client_score: Option<f64>,
    pub client_grade: Option<String>,
    /// The client's `ololo_health::HealthResult`, capped in size.
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub client_result: Option<Json>,
    pub client_duration_ms: Option<i64>,
    pub client_jscpd_version: Option<String>,
    pub client_error: Option<String>,
    pub client_reported_at: Option<ChronoDateTimeUtc>,
    /// `pending` | `ok` | `failed` | `timeout` | `commit_missing` | `unverified`.
    pub server_status: String,
    pub server_score: Option<f64>,
    pub server_grade: Option<String>,
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub server_result: Option<Json>,
    pub server_duration_ms: Option<i64>,
    pub server_jscpd_version: Option<String>,
    pub server_error: Option<String>,
    pub server_verified_at: Option<ChronoDateTimeUtc>,
    /// How the client's run of the project's tests after this probe ended
    /// (`ok` | `timeout` | `failed` | `declined`); NULL when none was
    /// reported for this commit — the checkpoint then counts the last run
    /// before it.
    pub tests_status: Option<String>,
    /// That run's `TestReportPayload`.
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub tests_result: Option<Json>,
    pub tests_reported_at: Option<ChronoDateTimeUtc>,
    pub created_at: ChronoDateTimeUtc,
    pub updated_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::sessions::Entity",
        from = "Column::SessionIdFk",
        to = "super::sessions::Column::Id",
        on_delete = "Cascade"
    )]
    Session,
    #[sea_orm(
        belongs_to = "super::players::Entity",
        from = "Column::PlayerIdFk",
        to = "super::players::Column::Id",
        on_delete = "Cascade"
    )]
    Player,
    #[sea_orm(
        belongs_to = "super::tasks::Entity",
        from = "Column::TaskIdFk",
        to = "super::tasks::Column::Id",
        on_delete = "SetNull"
    )]
    Task,
}

impl Related<super::sessions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Session.def()
    }
}

impl Related<super::players::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Player.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
