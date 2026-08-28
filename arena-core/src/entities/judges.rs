//! `judges` entity — admin-authored LLM evaluator definitions.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "judges")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub prompt: String,
    #[sea_orm(column_type = "Json")]
    pub rating_scale: Json,
    /// `"llm"` (default) or `"execution"`.
    pub kind: String,
    /// `"task"` (default) — runs per (player, task) when that task's commit
    /// lands — or `"session"`, which runs once per (player, session) and
    /// scores every task the player reached.
    pub scope: String,
    /// How the judge receives its facts: `"tools"` (default) runs the agentic
    /// loop with git tools; `"dossier"` runs one completion against an
    /// evidence pack built server-side, with no tools offered.
    pub evidence_mode: String,
    /// JSON array of the evidence sections this judge declared it needs (see
    /// `arena_core::judging::evidence::EvidenceNeeds`). NULL means the judge
    /// never declared, and gets the whole snapshot.
    pub evidence_needs: Option<String>,
    /// Optional per-judge model override: provider row + model id. Both NULL
    /// (and no pool) falls back to the `llm_op_judge` assignment, then
    /// `llm_default`. The two travel together — a model without a provider is
    /// not an override.
    pub llm_provider_id_fk: Option<Uuid>,
    pub llm_model: Option<String>,
    /// Optional per-judge pool override. May be set alongside the
    /// provider+model pair; `llm_source_order` then decides which is tried
    /// first and which is the failover behind it.
    pub llm_pool_id_fk: Option<Uuid>,
    /// `pool_first` (default) or `model_first` — see
    /// [`crate::llm::resolve::LlmOverride`].
    pub llm_source_order: String,
    /// JSON array of the open-ended criteria keys this judge scores. NULL =
    /// the judge scores no per-criterion sheet (classic single-rating).
    pub criteria: Option<String>,
    /// JSON array of repo path prefixes this judge's git tools must not open
    /// — the platform's `.ololo/` runtime tree, for a judge that only reads
    /// the player's own code. NULL = the whole snapshot is in scope.
    pub ignore_paths: Option<String>,
    /// Judge-declared probes (`crate::evaluation::JudgeProbeDef` array),
    /// materialized as `tests` rows with `initiator='judge'` when the judge
    /// is enqueued. NULL = the judge declares none.
    #[sea_orm(column_type = "Json", nullable)]
    pub probes_config: Option<Json>,
    /// How many interactive probes this judge may register per task. NULL =
    /// 0 (may register none).
    pub max_interactive: Option<i32>,
    /// Optional avatar for judge rows in the UI; admin-set in settings.
    pub avatar_url: Option<String>,
    pub created_at: ChronoDateTimeUtc,
    pub updated_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::task_judges::Entity")]
    TaskJudges,
}

impl Related<super::task_judges::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::TaskJudges.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
