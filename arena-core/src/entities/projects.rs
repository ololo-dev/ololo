//! `projects` entity. Spec: FR-001, FR-002, FR-015, FR-016.
//!
//! A project owns a task list. Sessions reference a project (NOT NULL).
//! `archived_at` implements soft-retire (NULL = active).
//! `public` controls discoverability by non-owner users.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "projects")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub name: String,
    pub slug: Option<String>,
    pub description: String,
    pub category: Option<String>,
    pub tags: String,
    pub cover_image_url: Option<String>,
    pub owner_user_id_fk: Uuid,
    pub public: bool,
    pub archived_at: Option<ChronoDateTimeUtc>,
    pub created_at: ChronoDateTimeUtc,
    pub updated_at: ChronoDateTimeUtc,
    pub default_value_points: i32,
    pub default_fail_points: i32,
    pub default_no_response_points: i32,
    pub default_completion_bonus_points: i32,
    pub default_deadline_secs: i64,
    pub default_session_duration_secs: i64,
    /// Cancel a running session after this many seconds with no connected
    /// agents. 0 disables the idle sweep for this project's sessions.
    pub idle_timeout_secs: i32,
    pub default_min_interval_secs: i32,
    pub default_interval_increment_secs: i32,
    pub default_max_interval_secs: i32,
    /// JSON object declaring the per-player session-memory keys and their
    /// default values, e.g. `{"dev": "npm run dev", "port": 1234}`. NULL
    /// means the project does not use session memory.
    pub memory_schema: Option<String>,
    /// Whether the public project page lists the task arc up front.
    /// Quiz-shaped projects (Extreme Startup family) stage their questions
    /// as a surprise and set this false in their seed frontmatter.
    pub show_tasks: bool,
    /// Campaign membership: the parent project this row is a part of.
    /// NULL for standalone projects and for campaign parents themselves.
    /// No DB-level FK (SQLite ALTER limitation) — integrity is enforced by
    /// seed linking and the project delete handler.
    pub parent_project_id_fk: Option<Uuid>,
    /// 0-based position of this part inside its campaign; NULL unless
    /// `parent_project_id_fk` is set. Unique per parent
    /// (`ux_projects_parent_part`).
    pub part_ordinal: Option<i32>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::users::Entity",
        from = "Column::OwnerUserIdFk",
        to = "super::users::Column::Id"
    )]
    Owner,
    #[sea_orm(has_many = "super::sessions::Entity")]
    Sessions,
    #[sea_orm(has_many = "super::tasks::Entity")]
    Tasks,
    /// Self-FK to the campaign parent. Deliberately no `Related` impl —
    /// `find_related` on a self-relation is directionally ambiguous; query
    /// by column (`Column::ParentProjectIdFk`) instead.
    #[sea_orm(
        belongs_to = "Entity",
        from = "Column::ParentProjectIdFk",
        to = "Column::Id"
    )]
    ParentProject,
}

impl Related<super::users::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Owner.def()
    }
}

impl Related<super::sessions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Sessions.def()
    }
}

impl Related<super::tasks::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Tasks.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
