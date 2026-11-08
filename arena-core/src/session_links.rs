//! Linked sessions — a few sessions a host runs as one event (the rounds of
//! a workshop, one brief for several rooms), read together: each session's
//! page names the others, the board flips between them, and once every one
//! of them is over there is one standings for the lot.
//!
//! The link is a `group_id` shared by the `session_links` rows of the
//! sessions in it. Linking two sessions merges whatever groups they were in;
//! unlinking one takes it out, and a group left with a single session is
//! dissolved — one session linked to nothing is not a group.
//!
//! The combined standings add up each player's points across the sessions,
//! one row per *user* (a player row is per session; the account is what
//! carries over). A player without an account stays a row of their own.
use std::collections::{BTreeMap, HashMap};

use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, Set,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entities::{players, session_links, users};

/// The group a session is linked into, if any.
pub async fn group_of(db: &DatabaseConnection, session_id: Uuid) -> Result<Option<Uuid>, DbErr> {
    Ok(session_links::Entity::find_by_id(session_id)
        .one(db)
        .await?
        .map(|r| r.group_id))
}

/// Every session in a group, in the order they were linked.
pub async fn sessions_in_group(
    db: &DatabaseConnection,
    group_id: Uuid,
) -> Result<Vec<Uuid>, DbErr> {
    let mut rows = session_links::Entity::find()
        .filter(session_links::Column::GroupId.eq(group_id))
        .all(db)
        .await?;
    rows.sort_by_key(|r| r.created_at);
    Ok(rows.into_iter().map(|r| r.session_id_fk).collect())
}

/// The sessions linked with this one, itself included — empty when it is
/// linked to nothing.
pub async fn linked_sessions(
    db: &DatabaseConnection,
    session_id: Uuid,
) -> Result<Vec<Uuid>, DbErr> {
    match group_of(db, session_id).await? {
        Some(g) => sessions_in_group(db, g).await,
        None => Ok(Vec::new()),
    }
}

/// Link two sessions: they end up in one group, together with everything
/// either was linked to before. Returns the group's id. Linking a session
/// to itself, or two sessions already linked, changes nothing.
pub async fn link(db: &DatabaseConnection, a: Uuid, b: Uuid) -> Result<Uuid, DbErr> {
    let ga = group_of(db, a).await?;
    let gb = group_of(db, b).await?;
    let now = Utc::now();
    match (ga, gb) {
        (Some(x), Some(y)) if x == y => Ok(x),
        (Some(x), Some(y)) => {
            // Two groups become one: the second one's rows move over.
            session_links::Entity::update_many()
                .col_expr(
                    session_links::Column::GroupId,
                    sea_orm::sea_query::Expr::value(x),
                )
                .filter(session_links::Column::GroupId.eq(y))
                .exec(db)
                .await?;
            Ok(x)
        }
        (Some(x), None) => {
            if a != b {
                insert_row(db, b, x, now).await?;
            }
            Ok(x)
        }
        (None, Some(y)) => {
            insert_row(db, a, y, now).await?;
            Ok(y)
        }
        (None, None) => {
            let g = Uuid::new_v4();
            if a == b {
                return Ok(g);
            }
            insert_row(db, a, g, now).await?;
            // A moment later, so the order of linking is the order shown.
            insert_row(db, b, g, now + chrono::Duration::milliseconds(1)).await?;
            Ok(g)
        }
    }
}

async fn insert_row(
    db: &DatabaseConnection,
    session_id: Uuid,
    group_id: Uuid,
    at: chrono::DateTime<Utc>,
) -> Result<(), DbErr> {
    session_links::ActiveModel {
        session_id_fk: Set(session_id),
        group_id: Set(group_id),
        created_at: Set(at),
    }
    .insert(db)
    .await?;
    Ok(())
}

/// Take a session out of its group. A group left with one session is
/// dissolved. A session linked to nothing is left alone.
pub async fn unlink(db: &DatabaseConnection, session_id: Uuid) -> Result<(), DbErr> {
    let Some(group) = group_of(db, session_id).await? else {
        return Ok(());
    };
    session_links::Entity::delete_by_id(session_id)
        .exec(db)
        .await?;
    let rest = sessions_in_group(db, group).await?;
    if rest.len() < 2 {
        session_links::Entity::delete_many()
            .filter(session_links::Column::GroupId.eq(group))
            .exec(db)
            .await?;
    }
    Ok(())
}

/// What one session contributed to a combined row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CombinedSessionPoints {
    pub session_id: Uuid,
    pub points: i64,
    /// The player's place in that session, 1-based.
    pub rank: u32,
}

/// One row of the combined standings of linked sessions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CombinedEntry {
    /// The account the row adds up, when the player has one.
    pub user_id: Option<Uuid>,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub total_points: i64,
    pub sessions_played: u32,
    /// Per-session contributions, in the order the sessions were given.
    pub per_session: Vec<CombinedSessionPoints>,
}

/// The standings across `session_ids`: every player's points added up over
/// the sessions they played, by account. Sorted by total, then by sessions
/// played, then by name — so a player who showed up for every round sits
/// above one who scored the same in fewer.
pub async fn combined_leaderboard(
    db: &DatabaseConnection,
    session_ids: &[Uuid],
) -> Result<Vec<CombinedEntry>, DbErr> {
    if session_ids.is_empty() {
        return Ok(Vec::new());
    }
    let player_rows = players::Entity::find()
        .filter(players::Column::SessionIdFk.is_in(session_ids.iter().copied()))
        .filter(players::Column::RevokedAt.is_null())
        .all(db)
        .await?;
    let user_of_player: HashMap<Uuid, Option<Uuid>> =
        player_rows.iter().map(|p| (p.id, p.user_id_fk)).collect();
    let mut user_ids: Vec<Uuid> = player_rows.iter().filter_map(|p| p.user_id_fk).collect();
    user_ids.sort_unstable();
    user_ids.dedup();
    let user_by_id: HashMap<Uuid, users::Model> = if user_ids.is_empty() {
        HashMap::new()
    } else {
        users::Entity::find()
            .filter(users::Column::Id.is_in(user_ids))
            .all(db)
            .await?
            .into_iter()
            .map(|u| (u.id, u))
            .collect()
    };

    // Rows keyed by account, or by the player row when there is none —
    // a BTreeMap so ties come out in a stable order.
    let mut rows: BTreeMap<Uuid, CombinedEntry> = BTreeMap::new();
    for &session_id in session_ids {
        let standings = crate::scoring::compute_leaderboard(db, session_id).await?;
        for (idx, entry) in standings.iter().enumerate() {
            let player_uuid = entry.player_id.as_uuid();
            let user_id = user_of_player.get(&player_uuid).copied().flatten();
            let key = user_id.unwrap_or(player_uuid);
            let row = rows.entry(key).or_insert_with(|| {
                let user = user_id.and_then(|id| user_by_id.get(&id));
                CombinedEntry {
                    user_id,
                    display_name: entry.display_name.clone(),
                    username: user.and_then(|u| u.username.clone()),
                    avatar_url: user.and_then(|u| u.avatar_url.clone()),
                    total_points: 0,
                    sessions_played: 0,
                    per_session: Vec::new(),
                }
            });
            row.total_points += entry.total_points;
            row.sessions_played += 1;
            row.per_session.push(CombinedSessionPoints {
                session_id,
                points: entry.total_points,
                rank: (idx + 1) as u32,
            });
        }
    }
    let mut out: Vec<CombinedEntry> = rows.into_values().collect();
    out.sort_by(|a, b| {
        b.total_points
            .cmp(&a.total_points)
            .then(b.sessions_played.cmp(&a.sessions_played))
            .then_with(|| {
                a.display_name
                    .to_lowercase()
                    .cmp(&b.display_name.to_lowercase())
            })
    });
    Ok(out)
}
