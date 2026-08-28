//! `users` entity. Spec: Migrations §1, FR-001, FR-002.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(unique)]
    pub email: String,
    pub password_hash: Option<String>,
    pub display_name: String,
    pub created_at: ChronoDateTimeUtc,
    pub updated_at: ChronoDateTimeUtc,
    /// Whether this user has admin privileges (FR-001). Set to `true` for
    /// the first registered user; `false` for all subsequent registrations.
    pub is_admin: bool,
    /// Optional avatar URL stored after an ImageKit upload (FR-004, NFR-006).
    /// `None` for users who have not set an avatar.
    pub avatar_url: Option<String>,
    /// Whether the user's email address has been verified.
    pub email_verified: bool,
    /// Unique lowercase username. Populated by migration backfill and
    /// auto-generated on new registrations. `None` only during migration window.
    pub username: Option<String>,
    /// Account plan: [`crate::quota::PLAN_FREE`] or
    /// [`crate::quota::PLAN_PREMIUM`]. Selects the tier judge-run limit.
    pub plan: String,
    /// Per-user monthly judge-run limit override. `None` = the plan's
    /// tier limit from `app_settings` applies.
    pub judge_run_limit: Option<i32>,
    /// Purchased judge-review credits (top-up packs). Spent only after the
    /// monthly allowance is exhausted; never resets with the month.
    pub judge_run_credits: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::refresh_tokens::Entity")]
    RefreshTokens,
    #[sea_orm(has_many = "super::players::Entity")]
    Players,
}

impl Related<super::refresh_tokens::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::RefreshTokens.def()
    }
}

impl Related<super::players::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Players.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
