//! Cross-session copy/paste validation, run once at session finish.
//!
//! Every finishing player's pushed HEAD is compared (jscpd) against the
//! repos of earlier finished sessions of the same project stored on this
//! game server — **other users' repos only**. The player's own past runs
//! are excluded from the penalised share: replaying a project converges on
//! your own earlier text almost by definition (code golf literally optimises
//! toward one spelling), and charging for it made practice runs
//! score-negative. Self-overlap is still measured and logged
//! ([`SimilarityReport::own_overlap_lines`]) — visible, never charged. The
//! share of the new code that duplicates the rest of the corpus, past a
//! configurable threshold ([`SIMILARITY_THRESHOLD_KEY`], default 25% —
//! identical briefs converge naturally, small ones especially), dampens the
//! player's game score linearly down to zero via a deterministic negative
//! `task_results` adjustment (`task_id` NULL, the same row shape completion
//! bonuses use, so every score surface — leaderboard, awards, ratings —
//! stays coherent). The full jscpd evidence lands in the session event log.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use arena_core::entities::{players, sessions, task_results};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use uuid::Uuid;

/// `app_settings` key: duplication percentage below which no penalty
/// applies. `100` disables the check entirely.
pub const SIMILARITY_THRESHOLD_KEY: &str = "similarity_threshold_pct";
pub const DEFAULT_SIMILARITY_THRESHOLD_PCT: u32 = 25;

/// Most recent corpus repos compared against; bounds jscpd wall time.
const MAX_CORPUS_REPOS: usize = 25;
/// Wall-clock cap for one player's scan (export + jscpd + parse).
const SCAN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);
/// jscpd clone floor — short snippets converge honestly.
const MIN_TOKENS: u32 = 50;
/// Marker in `task_results.answer`; also the idempotency key.
const PENALTY_MARKER: &str = "similarity-penalty";

/// One prior repo that contributed matches.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SimilaritySource {
    pub join_code: String,
    pub player: String,
    pub matched_lines: u64,
}

/// Outcome of one player's scan.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SimilarityReport {
    pub duplicated_pct: f64,
    pub duplicated_lines: u64,
    pub total_lines: u64,
    pub corpus_repos: usize,
    pub sources: Vec<SimilaritySource>,
    /// Candidate lines matching the player's OWN earlier sessions — measured
    /// for the record, excluded from `duplicated_lines`/`duplicated_pct` and
    /// thus from the penalty: a replay converging on your own past code is
    /// practice, not plagiarism.
    #[serde(default)]
    pub own_overlap_lines: u64,
}

/// The threshold, read from settings with the default as fallback.
async fn threshold_pct(db: &DatabaseConnection) -> u32 {
    use arena_core::entities::app_settings;
    app_settings::Entity::find()
        .filter(app_settings::Column::Key.eq(SIMILARITY_THRESHOLD_KEY))
        .one(db)
        .await
        .ok()
        .flatten()
        .and_then(|r| r.value.trim().parse::<u32>().ok())
        .filter(|n| *n <= 100)
        .unwrap_or(DEFAULT_SIMILARITY_THRESHOLD_PCT)
}

/// Proportional dampening: past the threshold the penalty is the duplicated
/// share of the CURRENT score — 35% copy/paste on 100 points costs 35.
/// Below the threshold (honest convergence) nothing is charged.
pub fn penalty_points(total_points: i64, duplicated_pct: f64, threshold_pct: u32) -> i64 {
    if total_points <= 0 || duplicated_pct <= f64::from(threshold_pct) || threshold_pct >= 100 {
        return 0;
    }
    let share = (duplicated_pct / 100.0).clamp(0.0, 1.0);
    -((total_points as f64 * share).round() as i64).min(total_points)
}

/// Merge (start, end) line intervals and count covered lines — overlapping
/// clones must not count the same line twice.
pub fn merged_line_count(mut intervals: Vec<(u64, u64)>) -> u64 {
    intervals.retain(|(s, e)| e >= s);
    intervals.sort_unstable();
    let mut covered = 0u64;
    let mut current: Option<(u64, u64)> = None;
    for (s, e) in intervals {
        match current {
            Some((cs, ce)) if s <= ce + 1 => current = Some((cs, ce.max(e))),
            Some((cs, ce)) => {
                covered += ce - cs + 1;
                current = Some((s, e));
                let _ = cs;
            }
            None => current = Some((s, e)),
        }
    }
    if let Some((cs, ce)) = current {
        covered += ce - cs + 1;
    }
    covered
}

/// Parse a jscpd JSON report into a [`SimilarityReport`].
///
/// Only clones that CROSS the candidate/corpus boundary count — duplication
/// inside the player's own new code is code-quality territory, not
/// plagiarism. `label_of` maps a corpus path to its source label. `is_own`
/// marks corpus paths belonging to the scanning player's earlier sessions:
/// their clones move to `own_overlap_lines` instead of the penalised share
/// (and are never listed as sources).
///
/// `total_lines` is OUR count of the candidate's text lines, not jscpd's:
/// jscpd's own percentage pools both sides of the comparison into the
/// denominator (an identical file reads ~50%, 100% is unreachable), and its
/// per-source statistics changed shape between major versions. Counting the
/// candidate ourselves keeps 0–100% meaning "share of the delivered code",
/// slightly conservative when the candidate holds files jscpd would skip.
pub fn parse_report(
    report: &serde_json::Value,
    candidate_prefix: &str,
    corpus_prefix: &str,
    label_of: &dyn Fn(&str) -> Option<(String, String)>,
    is_own: &dyn Fn(&str) -> bool,
    corpus_repos: usize,
    total_lines: u64,
) -> SimilarityReport {
    let in_candidate = |name: &str| {
        name.starts_with(candidate_prefix) || name.contains(&format!("/{candidate_prefix}"))
    };
    let in_corpus =
        |name: &str| name.starts_with(corpus_prefix) || name.contains(&format!("/{corpus_prefix}"));
    // `.ololo/` is platform plumbing (done-files, delivered artifacts,
    // memory notes) that every session legitimately shares — matches there
    // are not plagiarism. The scan's `--ignore` glob excludes it too, but
    // glob dot-dir semantics vary across jscpd versions; this filter is the
    // guarantee.
    let in_ololo = |name: &str| name.starts_with(".ololo/") || name.contains("/.ololo/");

    // Candidate-side clone intervals per file (own matches kept apart from
    // the penalised pool), and matched lines per source.
    let mut per_file: HashMap<String, Vec<(u64, u64)>> = HashMap::new();
    let mut per_file_own: HashMap<String, Vec<(u64, u64)>> = HashMap::new();
    let mut per_source: BTreeMap<(String, String), u64> = BTreeMap::new();
    for dup in report["duplicates"].as_array().into_iter().flatten() {
        let files = [&dup["firstFile"], &dup["secondFile"]];
        let names: Vec<&str> = files
            .iter()
            .map(|f| f["name"].as_str().unwrap_or(""))
            .collect();
        if names.iter().any(|n| in_ololo(n)) {
            continue;
        }
        let cand = files.iter().zip(&names).find(|(_, n)| in_candidate(n));
        let corp = names.iter().find(|n| in_corpus(n));
        let (Some((cand_file, cand_name)), Some(corp_name)) = (cand, corp) else {
            continue; // not a cross-boundary clone
        };
        let start = cand_file["startLoc"]["line"]
            .as_u64()
            .or_else(|| cand_file["start"].as_u64())
            .unwrap_or(0);
        let end = cand_file["endLoc"]["line"]
            .as_u64()
            .or_else(|| cand_file["end"].as_u64())
            .unwrap_or(start);
        if is_own(corp_name) {
            // The player's own earlier session: measured, never charged. A
            // block that ALSO matches someone else's repo arrives as its own
            // clone pair and still lands in the penalised pool below.
            per_file_own
                .entry((*cand_name).to_string())
                .or_default()
                .push((start, end));
            continue;
        }
        per_file
            .entry((*cand_name).to_string())
            .or_default()
            .push((start, end));
        if let Some(label) = label_of(corp_name) {
            *per_source.entry(label).or_default() += end.saturating_sub(start) + 1;
        }
    }

    let duplicated_lines: u64 = per_file.into_values().map(merged_line_count).sum();
    let own_overlap_lines: u64 = per_file_own.into_values().map(merged_line_count).sum();

    let mut sources: Vec<SimilaritySource> = per_source
        .into_iter()
        .map(|((join_code, player), matched_lines)| SimilaritySource {
            join_code,
            player,
            matched_lines,
        })
        .collect();
    sources.sort_by_key(|s| std::cmp::Reverse(s.matched_lines));
    sources.truncate(5);

    SimilarityReport {
        duplicated_pct: if total_lines == 0 {
            0.0
        } else {
            // Merged clone intervals can slightly overshoot a file's real
            // length on format edges; the share is still capped at 100.
            (100.0 * duplicated_lines as f64 / total_lines as f64).min(100.0)
        },
        duplicated_lines,
        total_lines,
        corpus_repos,
        sources,
        own_overlap_lines,
    }
}

/// Count the candidate's text lines — the denominator of the share.
/// Binary files (NUL byte in the first 4 KiB) and anything over 1 MiB are
/// skipped, matching what a clone detector could meaningfully match.
pub fn count_text_lines(dir: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path
                    .file_name()
                    .is_some_and(|n| n == "node_modules" || n == ".ololo")
                {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.len() > 1024 * 1024 {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            if bytes.iter().take(4096).any(|b| *b == 0) {
                continue; // binary
            }
            total += bytes.iter().filter(|b| **b == b'\n').count() as u64;
            if bytes.last().is_some_and(|b| *b != b'\n') && !bytes.is_empty() {
                total += 1; // unterminated final line still counts
            }
        }
    }
    total
}

/// Export a bare repo's HEAD into `dst`. `false` when there is no HEAD
/// (nothing was ever pushed) or the export fails.
async fn export_head(repo: &Path, dst: &Path) -> bool {
    if std::fs::create_dir_all(dst).is_err() {
        return false;
    }
    let repo = repo.to_path_buf();
    let dst = dst.to_path_buf();
    tokio::task::spawn_blocking(move || {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(format!(
                "git --git-dir {repo} archive HEAD | tar -x -C {dst}",
                repo = shlex::try_quote(&repo.to_string_lossy()).unwrap_or_default(),
                dst = shlex::try_quote(&dst.to_string_lossy()).unwrap_or_default(),
            ))
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

/// Run jscpd over the candidate and corpus dirs; parsed JSON report or None.
async fn run_jscpd(root: &Path) -> Option<serde_json::Value> {
    let root = root.to_path_buf();
    let out = tokio::task::spawn_blocking(move || {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(format!(
                // One root arg, cwd-relative: with `candidate corpus` as two
                // args jscpd reports names relative to EACH arg (bare
                // `app.js` vs `r0/app.js`) and the boundary is unmatchable;
                // scanning `.` yields stable `candidate/...` / `corpus/...`
                // prefixes on both jscpd v4 and v5.
                "cd {root} && jscpd --silent --reporters json --output .report \
                 --min-tokens {MIN_TOKENS} \
                 --ignore '**/node_modules/**,**/.ololo/**,**/*.lock,**/package-lock.json,**/.report/**' \
                 . >/dev/null 2>&1; cat .report/jscpd-report.json 2>/dev/null",
                root = shlex::try_quote(&root.to_string_lossy()).unwrap_or_default(),
            ))
            .output()
            .ok()
    })
    .await
    .ok()
    .flatten()?;
    serde_json::from_slice(&out.stdout).ok()
}

/// Run the cross-session scan for every linked player of a finished
/// session, persist evidence, and apply score dampening. Failures degrade
/// to "no penalty" — a missing jscpd binary or a broken export must never
/// cost anyone points.
pub async fn run_similarity_checks(
    state: &crate::state::GameServerState,
    session_id: Uuid,
    join_code: &str,
) {
    let db = &state.db;
    let threshold = threshold_pct(db).await;
    if threshold >= 100 {
        return;
    }
    let Ok(Some(session)) = sessions::Entity::find_by_id(session_id).one(db).await else {
        return;
    };
    let Some(repos_base) = arena_core::git_store::repos_base_dir() else {
        return;
    };

    // Corpus: earlier finished sessions of the same project, newest first.
    let Ok(prior_sessions) = sessions::Entity::find()
        .filter(sessions::Column::ProjectIdFk.eq(session.project_id_fk))
        .filter(sessions::Column::Id.ne(session_id))
        .filter(sessions::Column::FinishedAt.is_not_null())
        .order_by_desc(sessions::Column::FinishedAt)
        .limit(64)
        .all(db)
        .await
    else {
        return;
    };
    // (repo, join_code, player, owning user) — the owner is what lets each
    // scanning player's OWN earlier repos be measured without being charged.
    let mut corpus: Vec<(PathBuf, String, String, Option<Uuid>)> = Vec::new();
    for prior in &prior_sessions {
        if corpus.len() >= MAX_CORPUS_REPOS {
            break;
        }
        let Ok(prior_players) = players::Entity::find()
            .filter(players::Column::SessionIdFk.eq(prior.id))
            .all(db)
            .await
        else {
            continue;
        };
        for p in prior_players {
            if corpus.len() >= MAX_CORPUS_REPOS {
                break;
            }
            let repo = arena_core::git_store::player_repo_path(&repos_base, prior.id, p.id);
            if repo.join("HEAD").exists() {
                corpus.push((
                    repo,
                    prior.join_code.clone(),
                    p.display_name.clone(),
                    p.user_id_fk,
                ));
            }
        }
    }
    if corpus.is_empty() {
        return;
    }

    let Ok(session_players) = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(session_id))
        .filter(players::Column::RevokedAt.is_null())
        .all(db)
        .await
    else {
        return;
    };

    // Feed anchor: the project's final task (the activity table requires a
    // task FK; the similarity verdict is session-wide by nature).
    let last_task: Option<Uuid> = arena_core::entities::tasks::Entity::find()
        .filter(arena_core::entities::tasks::Column::ProjectIdFk.eq(session.project_id_fk))
        .order_by_desc(arena_core::entities::tasks::Column::Ordinal)
        .one(db)
        .await
        .ok()
        .flatten()
        .map(|t| t.id);

    let scratch = std::env::temp_dir().join(format!("ololo-similarity-{session_id}"));
    let _ = std::fs::remove_dir_all(&scratch);

    // Export the corpus once; every player of this session compares
    // against the same tree.
    let corpus_root = scratch.join("shared-corpus");
    let mut labels: HashMap<String, (String, String, Option<Uuid>)> = HashMap::new();
    let mut exported = 0usize;
    for (i, (repo, code, player, owner)) in corpus.iter().enumerate() {
        let label = format!("r{i}");
        if export_head(repo, &corpus_root.join(&label)).await {
            labels.insert(label, (code.clone(), player.clone(), *owner));
            exported += 1;
        }
    }
    if exported == 0 {
        let _ = std::fs::remove_dir_all(&scratch);
        return;
    }

    let scores = arena_core::scoring::aggregate_scores(db, session_id)
        .await
        .unwrap_or_default();

    for player in &session_players {
        if player.user_id_fk.is_none() {
            continue;
        }
        let repo = arena_core::git_store::player_repo_path(&repos_base, session_id, player.id);
        if !repo.join("HEAD").exists() {
            continue;
        }
        let report = tokio::time::timeout(
            SCAN_TIMEOUT,
            scan_one(
                &scratch,
                &corpus_root,
                &repo,
                player.id,
                player.user_id_fk,
                &labels,
                exported,
            ),
        )
        .await
        .ok()
        .flatten();
        let Some(report) = report else {
            tracing::warn!(session_id = %session_id, player_id = %player.id,
                "similarity: scan unavailable (jscpd missing, export failed, or timeout) — no penalty");
            // The absence of a report must itself be visible: an empty
            // repo or a broken scan otherwise looks identical to "never
            // ran" (session KN5JHB).
            crate::session_log_store::record(
                crate::session_log_store::base_dir(),
                session_id,
                Some(player.id),
                "similarity",
                serde_json::json!({
                    "player_id": player.id,
                    "status": "unavailable",
                    "reason": "scan unavailable: jscpd missing, repo export failed \
                               (e.g. no pushed commits), or timeout",
                }),
            )
            .await;
            continue;
        };

        let total = scores.get(&player.id).map(|s| s.total_points).unwrap_or(0);
        let penalty = penalty_points(total, report.duplicated_pct, threshold);
        crate::session_log_store::record(
            crate::session_log_store::base_dir(),
            session_id,
            Some(player.id),
            "similarity",
            serde_json::json!({
                "player_id": player.id,
                "report": report,
                "threshold_pct": threshold,
                "total_points": total,
                "penalty": penalty,
            }),
        )
        .await;
        // Clean runs included: the player page shows "checked, N%" instead
        // of silence, and the sources are named when there are any.
        let report_row = arena_core::entities::similarity_reports::ActiveModel {
            id: Set(Uuid::new_v4()),
            session_id_fk: Set(session_id),
            player_id_fk: Set(player.id),
            duplicated_pct: Set(report.duplicated_pct),
            duplicated_lines: Set(report.duplicated_lines as i64),
            total_lines: Set(report.total_lines as i64),
            corpus_repos: Set(report.corpus_repos as i32),
            penalty: Set(penalty),
            sources_json: Set(serde_json::to_value(&report.sources).unwrap_or_default()),
            created_at: Set(chrono::Utc::now()),
        };
        if let Err(e) = arena_core::entities::similarity_reports::Entity::insert(report_row)
            .on_conflict(
                sea_orm::sea_query::OnConflict::columns([
                    arena_core::entities::similarity_reports::Column::SessionIdFk,
                    arena_core::entities::similarity_reports::Column::PlayerIdFk,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec(db)
            .await
        {
            tracing::warn!(session_id = %session_id, error = %e, "similarity: report insert failed");
        }
        // The session dashboard's activity feed carries the verdict too.
        // Anchored to the project's final task purely to satisfy the feed's
        // task FK — the renderer keys off the kind and ignores the task.
        if let Some(last_task) = last_task {
            let activity = arena_core::entities::activity_event::ActiveModel {
                id: Set(Uuid::new_v4()),
                session_id_fk: Set(session_id),
                player_id_fk: Set(player.id),
                task_id_fk: Set(last_task),
                event_kind: Set("similarity".to_string()),
                task_ordinal: Set(0),
                task_title: Set(String::new()),
                player_display_name: Set(player.display_name.clone()),
                judge_name: Set(None),
                point_delta: Set(Some(penalty as i32)),
                detail: Set(Some(serde_json::json!({
                    "duplicated_pct": report.duplicated_pct,
                    "duplicated_lines": report.duplicated_lines,
                    "total_lines": report.total_lines,
                    "corpus_repos": report.corpus_repos,
                    // The feed can say HOW the number became a penalty (or
                    // did not) instead of leaving the rule implicit.
                    "threshold_pct": threshold,
                    // Overlap with the player's own earlier sessions —
                    // measured and shown, never part of the charged share.
                    "own_overlap_lines": report.own_overlap_lines,
                    // A passed check names nobody — under-threshold overlap
                    // is honest convergence, not an accusation to publish.
                    "sources": if penalty < 0 { report.sources.clone() } else { Vec::new() },
                }))),
                timestamp: Set(chrono::Utc::now()),
                version: Set(0),
            };
            if let Err(e) = arena_core::entities::activity_event::Entity::insert(activity)
                .exec(db)
                .await
            {
                tracing::warn!(session_id = %session_id, error = %e, "similarity: activity insert failed");
            }
        }
        tracing::info!(
            session_id = %session_id, join_code, player = %player.display_name,
            duplicated_pct = report.duplicated_pct, threshold, penalty,
            "similarity: scan complete"
        );
        if penalty < 0 {
            apply_penalty(db, session_id, player.id, penalty, &report).await;
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
}

/// One player's export + jscpd run + parse. `own_user` marks which corpus
/// repos are this player's earlier sessions — measured, never penalised.
async fn scan_one(
    scratch: &Path,
    corpus_root: &Path,
    repo: &Path,
    player_id: Uuid,
    own_user: Option<Uuid>,
    labels: &HashMap<String, (String, String, Option<Uuid>)>,
    corpus_repos: usize,
) -> Option<SimilarityReport> {
    let root = scratch.join(format!("scan-{player_id}"));
    let candidate = root.join("candidate");
    if !export_head(repo, &candidate).await {
        return None;
    }
    // The shared corpus is linked (or copied) under this scan's root so
    // jscpd sees exactly two top-level dirs with stable relative names.
    let corpus_link = root.join("corpus");
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(corpus_root, &corpus_link).is_ok();
    #[cfg(not(unix))]
    let linked = false;
    if !linked {
        return None;
    }
    let total_lines = {
        let candidate = candidate.clone();
        tokio::task::spawn_blocking(move || count_text_lines(&candidate))
            .await
            .unwrap_or(0)
    };
    let report = run_jscpd(&root).await?;
    let label_entry = |name: &str| -> Option<&(String, String, Option<Uuid>)> {
        let rest = name.split("corpus/").nth(1)?;
        let label = rest.split('/').next()?;
        labels.get(label)
    };
    let label_of = |name: &str| -> Option<(String, String)> {
        label_entry(name).map(|(code, player, _)| (code.clone(), player.clone()))
    };
    // Anonymous players (no user account) own nothing here: `None == None`
    // must not turn every unowned corpus repo into a free source.
    let is_own = |name: &str| -> bool {
        own_user.is_some() && label_entry(name).is_some_and(|(_, _, owner)| *owner == own_user)
    };
    Some(parse_report(
        &report,
        "candidate/",
        "corpus/",
        &label_of,
        &is_own,
        corpus_repos,
        total_lines,
    ))
}

/// The deterministic score adjustment: one `task_results` row (`task_id`
/// NULL, `is_bonus` true — the completion-bonus row shape), idempotent per
/// (session, player).
async fn apply_penalty(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    penalty: i64,
    report: &SimilarityReport,
) {
    use sea_orm::QueryFilter;
    let existing = task_results::Entity::find()
        .filter(task_results::Column::SessionIdFk.eq(session_id))
        .filter(task_results::Column::PlayerIdFk.eq(player_id))
        .filter(task_results::Column::IsBonus.eq(true))
        .filter(task_results::Column::TaskId.is_null())
        .all(db)
        .await
        .unwrap_or_default();
    if existing
        .iter()
        .any(|r| r.answer.starts_with(PENALTY_MARKER))
    {
        return;
    }
    // Name the top source: "whose code, in which session" is the difference
    // between an explanation and an accusation without evidence. The
    // player's own earlier sessions never appear here — they are excluded
    // from the penalised share before this point.
    let answer = match report.sources.first() {
        Some(src) => format!(
            "{PENALTY_MARKER}: {:.0}% of the delivered code matches {}'s code from session {}",
            report.duplicated_pct, src.player, src.join_code
        ),
        None => format!(
            "{PENALTY_MARKER}: {:.0}% of the delivered code duplicates earlier sessions of this project",
            report.duplicated_pct
        ),
    };
    if let Err(e) = (task_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(None),
        answer: Set(answer),
        created_at: Set(chrono::Utc::now()),
        point_delta: Set(penalty as i32),
        is_bonus: Set(true),
    }
    .insert(db)
    .await)
    {
        tracing::error!(session_id = %session_id, player_id = %player_id, error = %e,
            "similarity: penalty insert failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn penalty_is_zero_below_threshold_and_proportional_above() {
        assert_eq!(penalty_points(100, 10.0, 25), 0);
        assert_eq!(penalty_points(100, 25.0, 25), 0);
        // Past the threshold the duplicated share of the score is charged:
        // 35% copy/paste on 100 points costs 35.
        assert_eq!(penalty_points(100, 35.0, 25), -35);
        assert_eq!(penalty_points(183, 39.4, 25), -72);
        assert_eq!(penalty_points(100, 100.0, 25), -100);
        // Never rewards, never applies to non-positive scores, off at 100.
        assert_eq!(penalty_points(0, 90.0, 25), 0);
        assert_eq!(penalty_points(-40, 90.0, 25), 0);
        assert_eq!(penalty_points(100, 90.0, 100), 0);
    }

    #[test]
    fn merged_intervals_do_not_double_count() {
        assert_eq!(merged_line_count(vec![]), 0);
        assert_eq!(merged_line_count(vec![(1, 10)]), 10);
        assert_eq!(merged_line_count(vec![(1, 10), (5, 12)]), 12);
        assert_eq!(merged_line_count(vec![(1, 3), (10, 12)]), 6);
        // Adjacent ranges merge; inverted ranges are dropped.
        assert_eq!(merged_line_count(vec![(1, 3), (4, 6), (9, 2)]), 6);
    }

    #[test]
    fn report_counts_only_cross_boundary_clones() {
        let raw = serde_json::json!({
            "duplicates": [
                { // candidate ↔ corpus: counts
                    "firstFile": {"name": "candidate/app.js",
                                  "startLoc": {"line": 10}, "endLoc": {"line": 29}},
                    "secondFile": {"name": "corpus/r0/app.js",
                                   "startLoc": {"line": 5}, "endLoc": {"line": 24}}
                },
                { // overlap with the first clone: merged, not double-counted
                    "firstFile": {"name": "corpus/r1/app.js",
                                  "startLoc": {"line": 1}, "endLoc": {"line": 15}},
                    "secondFile": {"name": "candidate/app.js",
                                   "startLoc": {"line": 20}, "endLoc": {"line": 39}}
                },
                { // candidate-internal duplication: ignored
                    "firstFile": {"name": "candidate/a.js",
                                  "startLoc": {"line": 1}, "endLoc": {"line": 50}},
                    "secondFile": {"name": "candidate/b.js",
                                   "startLoc": {"line": 1}, "endLoc": {"line": 50}}
                },
                { // corpus-internal duplication: ignored
                    "firstFile": {"name": "corpus/r0/x.js",
                                  "startLoc": {"line": 1}, "endLoc": {"line": 50}},
                    "secondFile": {"name": "corpus/r1/x.js",
                                   "startLoc": {"line": 1}, "endLoc": {"line": 50}}
                }
            ],
        });
        let labels = |name: &str| -> Option<(String, String)> {
            let rest = name.split("corpus/").nth(1)?;
            match rest.split('/').next()? {
                "r0" => Some(("AAAAAA".into(), "alice".into())),
                "r1" => Some(("BBBBBB".into(), "bob".into())),
                _ => None,
            }
        };
        let report = parse_report(&raw, "candidate/", "corpus/", &labels, &|_| false, 2, 160);
        // Lines 10–29 and 20–39 of app.js merge into 10–39: 30 lines.
        assert_eq!(report.duplicated_lines, 30);
        assert_eq!(report.total_lines, 160);
        assert!((report.duplicated_pct - 18.75).abs() < 0.01);
        assert_eq!(report.sources.len(), 2);
        assert_eq!(report.sources[0].join_code, "AAAAAA");
        assert_eq!(report.own_overlap_lines, 0);
    }

    /// The replay regression: matches against the player's OWN earlier
    /// sessions are measured but never charged — practising a project (or
    /// re-golfing toward the same minimal spelling) must not be
    /// score-negative. A block that also matches someone ELSE stays charged.
    #[test]
    fn own_prior_sessions_are_measured_but_not_charged() {
        let raw = serde_json::json!({
            "duplicates": [
                { // candidate ↔ own replay (r0): excluded from the share
                    "firstFile": {"name": "candidate/answer.sh",
                                  "startLoc": {"line": 1}, "endLoc": {"line": 40}},
                    "secondFile": {"name": "corpus/r0/answer.sh",
                                   "startLoc": {"line": 1}, "endLoc": {"line": 40}}
                },
                { // the same block ALSO matches bob (r1): still charged
                    "firstFile": {"name": "candidate/answer.sh",
                                  "startLoc": {"line": 1}, "endLoc": {"line": 20}},
                    "secondFile": {"name": "corpus/r1/answer.sh",
                                   "startLoc": {"line": 1}, "endLoc": {"line": 20}}
                }
            ],
        });
        let labels = |name: &str| -> Option<(String, String)> {
            let rest = name.split("corpus/").nth(1)?;
            match rest.split('/').next()? {
                "r0" => Some(("MYPAST".into(), "me".into())),
                "r1" => Some(("THEIRS".into(), "bob".into())),
                _ => None,
            }
        };
        let is_own = |name: &str| name.contains("corpus/r0/");
        let report = parse_report(&raw, "candidate/", "corpus/", &labels, &is_own, 2, 100);
        assert_eq!(
            report.duplicated_lines, 20,
            "only the overlap with bob's repo is charged"
        );
        assert_eq!(
            report.own_overlap_lines, 40,
            "the self-overlap is still on the record"
        );
        assert_eq!(report.sources.len(), 1, "own sessions are never sources");
        assert_eq!(report.sources[0].join_code, "THEIRS");

        // A pure replay — every match is the player's own past run — must
        // carry zero penalty whatever the threshold.
        let only_own = |_: &str| true;
        let pure = parse_report(&raw, "candidate/", "corpus/", &labels, &only_own, 2, 100);
        assert_eq!(pure.duplicated_lines, 0);
        assert_eq!(pure.duplicated_pct, 0.0);
        assert_eq!(pure.own_overlap_lines, 40);
        assert_eq!(penalty_points(500, pure.duplicated_pct, 25), 0);
    }

    #[test]
    fn ololo_platform_files_never_count_as_plagiarism() {
        let raw = serde_json::json!({
            "duplicates": [
                { // done-files / artifacts plumbing on either side: dropped
                    "firstFile": {"name": "candidate/.ololo/weather-widget-done.md",
                                  "startLoc": {"line": 1}, "endLoc": {"line": 40}},
                    "secondFile": {"name": "corpus/r0/.ololo/weather-widget-done.md",
                                   "startLoc": {"line": 1}, "endLoc": {"line": 40}}
                },
                { // one .ololo side is enough to drop the clone
                    "firstFile": {"name": "candidate/app.js",
                                  "startLoc": {"line": 1}, "endLoc": {"line": 40}},
                    "secondFile": {"name": "corpus/r0/.ololo/notes.md",
                                   "startLoc": {"line": 1}, "endLoc": {"line": 40}}
                }
            ],
        });
        let labels =
            |_: &str| -> Option<(String, String)> { Some(("AAAAAA".into(), "alice".into())) };
        let report = parse_report(&raw, "candidate/", "corpus/", &labels, &|_| false, 1, 100);
        assert_eq!(report.duplicated_lines, 0);
        assert!(report.sources.is_empty());
    }

    use crate::test_fixtures::{mem_db, session_with_player};

    #[tokio::test]
    async fn threshold_reads_setting_and_rejects_nonsense() {
        use arena_core::entities::app_settings;
        use sea_orm::ActiveModelTrait;
        let db = mem_db().await;
        // No row → the shipped default.
        assert_eq!(threshold_pct(&db).await, DEFAULT_SIMILARITY_THRESHOLD_PCT);
        // A configured value wins.
        (app_settings::ActiveModel {
            key: Set(SIMILARITY_THRESHOLD_KEY.to_string()),
            value: Set(" 40 ".to_string()),
        })
        .insert(&db)
        .await
        .expect("insert setting");
        assert_eq!(threshold_pct(&db).await, 40);
        // Out-of-range or unparsable values fall back to the default.
        for bad in ["140", "not-a-number"] {
            app_settings::Entity::update(app_settings::ActiveModel {
                key: Set(SIMILARITY_THRESHOLD_KEY.to_string()),
                value: Set(bad.to_string()),
            })
            .exec(&db)
            .await
            .expect("update setting");
            assert_eq!(
                threshold_pct(&db).await,
                DEFAULT_SIMILARITY_THRESHOLD_PCT,
                "bad value {bad:?} must not survive"
            );
        }
    }

    #[tokio::test]
    async fn penalty_row_names_the_source_and_is_applied_once() {
        use sea_orm::{ColumnTrait, QueryFilter};
        let db = mem_db().await;
        let fx = session_with_player(&db).await;
        let (session_id, player_id) = (fx.session_id, fx.player_id);
        let report = SimilarityReport {
            duplicated_pct: 39.4,
            duplicated_lines: 72,
            total_lines: 183,
            corpus_repos: 2,
            sources: vec![SimilaritySource {
                join_code: "AAAAAA".to_string(),
                player: "alice".to_string(),
                matched_lines: 72,
            }],
            own_overlap_lines: 0,
        };
        apply_penalty(&db, session_id, player_id, -72, &report).await;
        // Re-running (a re-finish, a recovered session) must not double-charge.
        apply_penalty(&db, session_id, player_id, -72, &report).await;

        let rows = task_results::Entity::find()
            .filter(task_results::Column::SessionIdFk.eq(session_id))
            .all(&db)
            .await
            .expect("query");
        assert_eq!(rows.len(), 1, "the penalty is applied exactly once");
        let row = &rows[0];
        assert_eq!(row.point_delta, -72);
        assert!(row.is_bonus);
        assert!(row.answer.starts_with(PENALTY_MARKER));
        assert!(
            row.answer.contains("alice") && row.answer.contains("AAAAAA"),
            "the verdict names whose code and which session: {}",
            row.answer
        );
    }

    #[tokio::test]
    async fn sourceless_penalty_still_explains_itself() {
        use sea_orm::{ColumnTrait, QueryFilter};
        let db = mem_db().await;
        let fx = session_with_player(&db).await;
        let (session_id, player_id) = (fx.session_id, fx.player_id);
        // Sources can end up empty (labels unresolved) while the share is
        // real; the verdict must still say what happened.
        let report = SimilarityReport {
            duplicated_pct: 60.0,
            duplicated_lines: 60,
            total_lines: 100,
            corpus_repos: 1,
            sources: vec![],
            own_overlap_lines: 0,
        };
        apply_penalty(&db, session_id, player_id, -60, &report).await;
        let rows = task_results::Entity::find()
            .filter(task_results::Column::PlayerIdFk.eq(player_id))
            .all(&db)
            .await
            .expect("query");
        assert!(
            rows[0]
                .answer
                .contains("duplicates earlier sessions of this project"),
            "the sourceless verdict still explains the charge: {}",
            rows[0].answer
        );
    }
}
