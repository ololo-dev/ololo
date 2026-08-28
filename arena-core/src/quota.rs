//! Account plans and the monthly judge-run quota.
//!
//! Every user is on a plan — [`PLAN_FREE`] or [`PLAN_PREMIUM`] — and each
//! plan carries a monthly judge-run limit configured in `app_settings`
//! ([`FREE_JUDGE_RUN_LIMIT_KEY`] / [`PREMIUM_JUDGE_RUN_LIMIT_KEY`], defaults
//! [`DEFAULT_FREE_JUDGE_RUN_LIMIT`] / [`DEFAULT_PREMIUM_JUDGE_RUN_LIMIT`]).
//! A per-user `users.judge_run_limit` override wins over the tier limit.
//!
//! Usage is metered in `judge_run_ledger`: one row per run of the judge
//! pipeline (retries within a run are one unit), charged to the judged
//! player's user account, counted over the current calendar month (UTC).
//! The gate lives in the game server's judge dispatch — see
//! [`check_and_charge_judge_run`].

use crate::entities::{app_settings, judge_run_ledger, players, users};
use chrono::{DateTime, Datelike, TimeZone, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, Set,
};
use uuid::Uuid;

pub const PLAN_FREE: &str = "free";
pub const PLAN_PREMIUM: &str = "premium";

/// `app_settings` key of the master switch. Tiers are OFF unless this is
/// explicitly `"true"`: usage is still metered (the ledger keeps filling so
/// flipping the switch later starts from real numbers), but nothing is ever
/// denied and plan UI stays hidden.
pub const PLANS_ENABLED_KEY: &str = "plans_enabled";

/// `app_settings` keys holding the per-tier monthly judge-run limits.
pub const FREE_JUDGE_RUN_LIMIT_KEY: &str = "plan_free_judge_run_limit";
pub const PREMIUM_JUDGE_RUN_LIMIT_KEY: &str = "plan_premium_judge_run_limit";

pub const DEFAULT_FREE_JUDGE_RUN_LIMIT: i64 = 100;
pub const DEFAULT_PREMIUM_JUDGE_RUN_LIMIT: i64 = 1000;

// Display prices and purchasable review packs live in the server's
// `pricing` module: metering is meaningful on any deployment, selling runs
// is not, and the core must not carry a storefront.

/// A user's judge-run quota standing for the current calendar month.
#[derive(Debug, Clone)]
pub struct JudgeQuota {
    pub plan: String,
    pub used: i64,
    pub limit: i64,
    /// Purchased pack credits still unspent (not part of `limit`).
    pub credits: i64,
}

impl JudgeQuota {
    pub fn exhausted(&self) -> bool {
        self.used >= self.limit && self.credits <= 0
    }
}

/// Whether tiers are enforced. Absent row or any value other than `"true"`
/// → `false` (fail-open: the switch has to be thrown deliberately).
pub async fn plans_enabled<C: ConnectionTrait>(db: &C) -> Result<bool, sea_orm::DbErr> {
    let row = app_settings::Entity::find()
        .filter(app_settings::Column::Key.eq(PLANS_ENABLED_KEY))
        .one(db)
        .await?;
    Ok(row.is_some_and(|r| r.value == "true"))
}

/// Start of `now`'s calendar month (UTC) — the metering window boundary.
pub fn month_start_utc(now: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .expect("first of month midnight is always a valid UTC timestamp")
}

/// The monthly judge-run limit for `plan`, read from `app_settings` with the
/// built-in default as fallback (absent row or unparseable value).
pub async fn plan_judge_run_limit<C: ConnectionTrait>(
    db: &C,
    plan: &str,
) -> Result<i64, sea_orm::DbErr> {
    let (key, default) = if plan == PLAN_FREE {
        (FREE_JUDGE_RUN_LIMIT_KEY, DEFAULT_FREE_JUDGE_RUN_LIMIT)
    } else {
        (PREMIUM_JUDGE_RUN_LIMIT_KEY, DEFAULT_PREMIUM_JUDGE_RUN_LIMIT)
    };
    let row = app_settings::Entity::find()
        .filter(app_settings::Column::Key.eq(key))
        .one(db)
        .await?;
    Ok(row
        .and_then(|r| r.value.trim().parse::<i64>().ok())
        .filter(|n| *n >= 0)
        .unwrap_or(default))
}

/// The user's quota standing this month: effective limit (per-user override
/// over tier limit) and ledger usage since the month started.
pub async fn judge_quota_for_user<C: ConnectionTrait>(
    db: &C,
    user: &users::Model,
) -> Result<JudgeQuota, sea_orm::DbErr> {
    let limit = match user.judge_run_limit {
        Some(n) => i64::from(n),
        None => plan_judge_run_limit(db, &user.plan).await?,
    };
    let used = judge_runs_used_since(db, user.id, month_start_utc(Utc::now())).await?;
    Ok(JudgeQuota {
        plan: user.plan.clone(),
        used,
        limit,
        credits: user.judge_run_credits,
    })
}

/// Ledger rows charged to `user_id` since `since`.
pub async fn judge_runs_used_since<C: ConnectionTrait>(
    db: &C,
    user_id: Uuid,
    since: DateTime<Utc>,
) -> Result<i64, sea_orm::DbErr> {
    let n = judge_run_ledger::Entity::find()
        .filter(judge_run_ledger::Column::UserIdFk.eq(user_id))
        .filter(judge_run_ledger::Column::CreatedAt.gte(since))
        .count(db)
        .await?;
    Ok(n as i64)
}

/// Check the quota of the user behind `player_id` and, if there is headroom,
/// charge one run to the ledger.
///
/// Returns `Ok(None)` when the run may proceed — either a unit was charged,
/// or the player is unmetered (no row / no linked account; production join
/// paths always link one, so that's a legacy-data case, not a loophole).
/// Returns `Ok(Some(quota))` when the limit is reached and the run must not
/// start. Concurrent runs may slightly overshoot the limit — the check and
/// the charge are not one atomic statement, and that's acceptable metering
/// slack, not a billing invariant.
pub async fn check_and_charge_judge_run<C: ConnectionTrait>(
    db: &C,
    session_id: Uuid,
    player_id: Uuid,
    judge_id: Uuid,
) -> Result<Option<JudgeQuota>, sea_orm::DbErr> {
    let Some(player) = players::Entity::find_by_id(player_id).one(db).await? else {
        return Ok(None);
    };
    let Some(user_id) = player.user_id_fk else {
        return Ok(None);
    };
    check_and_charge_for_user(db, user_id, session_id, player_id, judge_id).await
}

/// [`check_and_charge_judge_run`] once the user is already resolved.
pub async fn check_and_charge_for_user<C: ConnectionTrait>(
    db: &C,
    user_id: Uuid,
    session_id: Uuid,
    player_id: Uuid,
    judge_id: Uuid,
) -> Result<Option<JudgeQuota>, sea_orm::DbErr> {
    let Some(user) = users::Entity::find_by_id(user_id).one(db).await? else {
        return Ok(None);
    };
    // With tiers disabled the run is still metered but never denied — the
    // ledger keeps real numbers for the day the switch is thrown.
    let mut source = "monthly";
    if plans_enabled(db).await? {
        let quota = judge_quota_for_user(db, &user).await?;
        if quota.used >= quota.limit {
            // The monthly allowance is gone; purchased pack credits carry
            // the run. The guarded UPDATE is the arbiter under concurrency:
            // it only decrements a positive balance, so two racing runs
            // cannot spend the same last credit.
            let spent = users::Entity::update_many()
                .col_expr(
                    users::Column::JudgeRunCredits,
                    Expr::col(users::Column::JudgeRunCredits).sub(1),
                )
                .filter(users::Column::Id.eq(user_id))
                .filter(users::Column::JudgeRunCredits.gt(0))
                .exec(db)
                .await?
                .rows_affected
                > 0;
            if !spent {
                return Ok(Some(quota));
            }
            source = "pack";
        }
    }
    judge_run_ledger::ActiveModel {
        id: Set(Uuid::new_v4()),
        user_id_fk: Set(user_id),
        session_id: Set(session_id),
        player_id: Set(player_id),
        judge_id: Set(judge_id),
        created_at: Set(Utc::now()),
        source: Set(source.to_string()),
    }
    .insert(db)
    .await?;
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use migration::MigratorTrait;
    use sea_orm::DatabaseConnection;

    async fn setup_db() -> DatabaseConnection {
        let db = sea_orm::Database::connect("sqlite::memory:")
            .await
            .expect("connect");
        migration::Migrator::up(&db, None).await.expect("migrate");
        db
    }

    async fn insert_user(
        db: &DatabaseConnection,
        plan: &str,
        judge_run_limit: Option<i32>,
    ) -> Uuid {
        insert_user_with_credits(db, plan, judge_run_limit, 0).await
    }

    async fn insert_user_with_credits(
        db: &DatabaseConnection,
        plan: &str,
        judge_run_limit: Option<i32>,
        credits: i64,
    ) -> Uuid {
        crate::entities::users::ActiveModel {
            id: Set(Uuid::new_v4()),
            email: Set(format!("u{}@example.com", Uuid::new_v4())),
            password_hash: Set(None),
            display_name: Set("tester".to_string()),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            is_admin: Set(false),
            avatar_url: Set(None),
            email_verified: Set(false),
            username: Set(None),
            plan: Set(plan.to_string()),
            judge_run_limit: Set(judge_run_limit),
            judge_run_credits: Set(credits),
        }
        .insert(db)
        .await
        .expect("insert user")
        .id
    }

    async fn set_setting(db: &DatabaseConnection, key: &str, value: &str) {
        app_settings::ActiveModel {
            key: Set(key.to_string()),
            value: Set(value.to_string()),
        }
        .insert(db)
        .await
        .expect("insert setting");
    }

    async fn charge(db: &DatabaseConnection, user_id: Uuid) -> Option<JudgeQuota> {
        check_and_charge_for_user(db, user_id, Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4())
            .await
            .expect("check_and_charge")
    }

    #[test]
    fn month_start_is_first_of_month_midnight() {
        let now = Utc.with_ymd_and_hms(2026, 8, 12, 15, 30, 45).unwrap();
        assert_eq!(
            month_start_utc(now),
            Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap()
        );
    }

    #[tokio::test]
    async fn tier_limits_fall_back_to_defaults() {
        let db = setup_db().await;
        assert_eq!(
            plan_judge_run_limit(&db, PLAN_FREE).await.unwrap(),
            DEFAULT_FREE_JUDGE_RUN_LIMIT
        );
        assert_eq!(
            plan_judge_run_limit(&db, PLAN_PREMIUM).await.unwrap(),
            DEFAULT_PREMIUM_JUDGE_RUN_LIMIT
        );
        set_setting(&db, FREE_JUDGE_RUN_LIMIT_KEY, "7").await;
        assert_eq!(plan_judge_run_limit(&db, PLAN_FREE).await.unwrap(), 7);
        // Garbage value → default, not zero.
        set_setting(&db, PREMIUM_JUDGE_RUN_LIMIT_KEY, "lots").await;
        assert_eq!(
            plan_judge_run_limit(&db, PLAN_PREMIUM).await.unwrap(),
            DEFAULT_PREMIUM_JUDGE_RUN_LIMIT
        );
    }

    #[tokio::test]
    async fn charges_until_tier_limit_then_denies() {
        let db = setup_db().await;
        set_setting(&db, PLANS_ENABLED_KEY, "true").await;
        set_setting(&db, FREE_JUDGE_RUN_LIMIT_KEY, "2").await;
        let uid = insert_user(&db, PLAN_FREE, None).await;

        assert!(charge(&db, uid).await.is_none());
        assert!(charge(&db, uid).await.is_none());
        let denied = charge(&db, uid).await.expect("third run denied");
        assert_eq!((denied.used, denied.limit), (2, 2));
        assert_eq!(denied.plan, PLAN_FREE);
        // A denied run charges nothing.
        assert_eq!(
            judge_runs_used_since(&db, uid, month_start_utc(Utc::now()))
                .await
                .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn per_user_override_beats_tier_limit() {
        let db = setup_db().await;
        set_setting(&db, PLANS_ENABLED_KEY, "true").await;
        set_setting(&db, FREE_JUDGE_RUN_LIMIT_KEY, "100").await;
        let uid = insert_user(&db, PLAN_FREE, Some(1)).await;

        assert!(charge(&db, uid).await.is_none());
        let denied = charge(&db, uid).await.expect("second run denied");
        assert_eq!((denied.used, denied.limit), (1, 1));
    }

    #[tokio::test]
    async fn disabled_plans_meter_but_never_deny() {
        let db = setup_db().await;
        // No plans_enabled row at all: the default is off.
        let uid = insert_user(&db, PLAN_FREE, Some(1)).await;

        assert!(charge(&db, uid).await.is_none());
        assert!(
            charge(&db, uid).await.is_none(),
            "over the limit, still allowed"
        );
        // Usage was metered the whole time.
        assert_eq!(
            judge_runs_used_since(&db, uid, month_start_utc(Utc::now()))
                .await
                .unwrap(),
            2
        );
        // The switch takes effect immediately.
        set_setting(&db, PLANS_ENABLED_KEY, "true").await;
        assert!(charge(&db, uid).await.is_some(), "enabled → denied");
    }

    #[tokio::test]
    async fn last_months_runs_do_not_count() {
        let db = setup_db().await;
        set_setting(&db, PLANS_ENABLED_KEY, "true").await;
        let uid = insert_user(&db, PLAN_FREE, Some(1)).await;
        // A run charged well before this month's window opened.
        judge_run_ledger::ActiveModel {
            id: Set(Uuid::new_v4()),
            user_id_fk: Set(uid),
            session_id: Set(Uuid::new_v4()),
            player_id: Set(Uuid::new_v4()),
            judge_id: Set(Uuid::new_v4()),
            created_at: Set(month_start_utc(Utc::now()) - chrono::Duration::hours(1)),
            source: Set("monthly".to_string()),
        }
        .insert(&db)
        .await
        .expect("insert old ledger row");

        assert!(charge(&db, uid).await.is_none());
    }

    #[tokio::test]
    async fn pack_credits_carry_runs_past_the_monthly_limit() {
        let db = setup_db().await;
        set_setting(&db, PLANS_ENABLED_KEY, "true").await;
        let uid = insert_user_with_credits(&db, PLAN_FREE, Some(1), 2).await;

        // 1 monthly + 2 pack credits = three runs, then denial.
        assert!(charge(&db, uid).await.is_none());
        assert!(charge(&db, uid).await.is_none());
        assert!(charge(&db, uid).await.is_none());
        let denied = charge(&db, uid).await.expect("fourth run denied");
        assert_eq!(denied.credits, 0);

        let user = crate::entities::users::Entity::find_by_id(uid)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(user.judge_run_credits, 0);
        let pack_runs = judge_run_ledger::Entity::find()
            .filter(judge_run_ledger::Column::UserIdFk.eq(uid))
            .filter(judge_run_ledger::Column::Source.eq("pack"))
            .count(&db)
            .await
            .unwrap();
        assert_eq!(pack_runs, 2);
    }

    #[tokio::test]
    async fn unknown_player_is_unmetered() {
        let db = setup_db().await;
        let allowed =
            check_and_charge_judge_run(&db, Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4())
                .await
                .expect("check");
        assert!(allowed.is_none());
    }
}
