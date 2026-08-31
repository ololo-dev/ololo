//! Session-scoped **report** run: one narrative written for the player at the
//! end of their session.
//!
//! Every other judge answers a question about one task and moves a score. This
//! one answers a question about the session: what did you build, where did it
//! fight you, what did the panel say, and what would be worth doing
//! differently next time. It moves no score at all — a report that could take
//! points away would be read as a verdict, and read defensively.
//!
//! ## Why it is a judge and not a separate subsystem
//!
//! It needs exactly what the judging path already provides: a resolved model,
//! quota metering, telemetry, the session dossier, and a terminal
//! `judge_results` row the settle poll can wait on. Riding the session path
//! also puts the text on the player page for free, because the player snapshot
//! already renders `judge_results.feedback`.
//!
//! ## Why it is a sibling of `session::run_session_judge` and not a branch
//!
//! That runner is anti-cheat-shaped in ways a reporter cannot inherit: it
//! derives its rating scale from what each task *paid* (so a task that earned
//! nothing never reaches the model), asks one narrow question per task, and
//! parses a JSON verdict. A reporter makes one call for the whole session and
//! returns prose. Sharing the entry point but not the body keeps that path —
//! which is load-bearing and subtle — untouched.

use std::collections::HashMap;
use std::path::Path;

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};
use uuid::Uuid;

use crate::entities::{judge_results, judges, task_judges, tasks};
use crate::validation::judge_results::validate_feedback;

use super::dossier::build_session_dossier;
use super::session::{SessionJudgeOutput, TaskVerdict};
use super::{AgentResponse, JudgeError, JudgeRow, TaskJudgeRow, persist_scored_verdict};

/// The player's session report, as the reporter writes it.
///
/// Structured rather than prose because the page renders more than words
/// around it: the completed tasks it lists come with their titles, points and
/// delivered artifacts from the server, and each judge's two lines sit in
/// their own cells. A free-markdown report could not be laid out that way
/// without guessing at its headings.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct SessionReportDoc {
    pub built: BuiltSection,
    /// Tasks that gave the player a hard time, worst first. Empty on a clean
    /// session — the absence is the finding.
    #[serde(default)]
    pub friction: Vec<FrictionItem>,
    /// One entry per scoring judge that had something to say.
    #[serde(default)]
    pub judges: Vec<JudgeNote>,
    /// One entry per criterion the panel scored, summarising the whole
    /// session rather than any one task. The scorecard shows a criterion
    /// once — averaged over the tasks it was scored on — and this is the
    /// description under it; without it the page can only fall back to the
    /// panel's own note from the last task, which is a reading, not a summary.
    #[serde(default)]
    pub criteria: Vec<CriterionNote>,
    /// Concrete changes, ordered by what would have earned the most here.
    #[serde(default)]
    pub improve: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct BuiltSection {
    pub brief: String,
    /// One line per completed task; the page supplies title and points.
    #[serde(default)]
    pub tasks: Vec<BuiltTask>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct BuiltTask {
    pub ordinal: i32,
    pub note: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct FrictionItem {
    pub ordinal: i32,
    pub what_happened: String,
    /// What the evidence suggests was behind it; absent when the record is
    /// silent — an honest blank beats a guess.
    #[serde(default)]
    pub why: Option<String>,
}

/// The reporter's session-wide word on one criterion, keyed by the criterion
/// key exactly as the evidence spells it — the page matches on it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct CriterionNote {
    pub key: String,
    pub summary: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct JudgeNote {
    pub judge: String,
    pub good: String,
    /// Absent when that judge asked for nothing.
    #[serde(default)]
    pub improve: Option<String>,
}

/// Parse the model's answer into a report document.
///
/// Tolerant of the two things models do to JSON: wrapping it in a markdown
/// fence, and prefixing it with a sentence. Anything that still will not parse
/// is returned as `None`, and the caller keeps the raw text — a report that
/// renders as plain prose beats no report at all.
pub fn parse_report_doc(raw: &str) -> Option<SessionReportDoc> {
    let text = raw.trim();
    let unfenced = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```"))
        .map(|rest| rest.trim_start().trim_end_matches("```").trim())
        .unwrap_or(text);
    let start = unfenced.find('{')?;
    let end = unfenced.rfind('}')?;
    if end <= start {
        return None;
    }
    let mut doc: SessionReportDoc = serde_json::from_str(&unfenced[start..=end]).ok()?;
    if doc.built.brief.trim().is_empty() {
        return None;
    }
    normalize(&mut doc);
    Some(doc)
}

/// How many rough patches the report shows. The prompt asks for this many;
/// this is what makes it so.
const MAX_FRICTION: usize = 3;

/// The things a model does that no prompt reliably stops, fixed here so the
/// page never has to show them.
///
/// It writes "probe" — our word for a task's check, which it can read in the
/// evidence and which means nothing to a player. It answers an optional field
/// with the *word* "None" instead of leaving it out, which then renders as a
/// cell reading "None". And it files every task that ever failed an attempt
/// under friction, however lightly.
fn normalize(doc: &mut SessionReportDoc) {
    scrub(&mut doc.built.brief);
    for t in &mut doc.built.tasks {
        scrub(&mut t.note);
    }
    // The prompt asks for at most three, worst first. A weaker model files
    // every task that ever failed an attempt — eight of ten, in the run this
    // was written against — which buries the one that actually cost them.
    // Keeping the model's own ordering, take it at its word about the top.
    doc.friction.truncate(MAX_FRICTION);
    for f in &mut doc.friction {
        scrub(&mut f.what_happened);
        blank_to_none(&mut f.why);
    }
    for j in &mut doc.judges {
        scrub(&mut j.judge);
        scrub(&mut j.good);
        blank_to_none(&mut j.improve);
    }
    for c in &mut doc.criteria {
        scrub(&mut c.summary);
    }
    // A key the page cannot match, or an empty summary, would render as a
    // criterion with no description at all — drop it and let the fallback run.
    doc.criteria
        .retain(|c| !c.key.trim().is_empty() && !c.summary.trim().is_empty());
    for s in &mut doc.improve {
        scrub(s);
    }
    doc.improve.retain(|s| !s.trim().is_empty());
}

/// The evidence's field and flag names, and what each one means to someone
/// who never read our schema.
///
/// The prompt already forbids these — and prod reports carried them anyway:
/// three of the four Money Tracker debriefs of 2026-08-22 explained a task
/// with "the task is flagged with no_task_commit and no_agent_stats". A model
/// reading a JSON dossier quotes its keys; the rule only holds if the server
/// makes it hold.
///
/// Longest first: `no_task_commit` is a suffix of nothing here, but
/// `empty_task_commit` and `passed_without_changes` overlap in spirit and the
/// order is what keeps a partial match from firing first.
const JARGON: &[(&str, &str)] = &[
    ("passed_without_changes", "a pass with no code change"),
    ("zero_agent_activity", "no recorded agent activity"),
    ("empty_task_commit", "a commit that changed nothing"),
    ("no_task_commit", "no commit for that task"),
    ("no_agent_stats", "no reported agent activity"),
    ("commit_sha is null", "there is no commit for the task"),
    ("commit_sha", "the task's commit"),
    ("point_delta", "the points it moved"),
    ("task_ordinal", "the task number"),
    ("diff stat", "what the commit changed"),
];

/// Our vocabulary out, the player's in. Case and plural preserved.
fn scrub(text: &mut String) {
    for (needle, plain) in JARGON {
        replace_ci(text, needle, plain);
    }
    if !text.to_ascii_lowercase().contains("probe") {
        return;
    }
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let rest = &text[i..];
        let head: String = rest.chars().take(5).collect();
        if head.eq_ignore_ascii_case("probe") {
            let capitalized = head.starts_with('P');
            out.push_str(if capitalized { "Check" } else { "check" });
            i += head.len();
        } else {
            let c = rest.chars().next().expect("non-empty");
            out.push(c);
            i += c.len_utf8();
        }
    }
    *text = out;
}

/// Case-insensitive literal replacement, in place. Nothing here needs a
/// regex: the needles are our own identifiers, and a model that mentions one
/// writes it exactly as the JSON did.
fn replace_ci(text: &mut String, needle: &str, plain: &str) {
    let lower = text.to_ascii_lowercase();
    if !lower.contains(needle) {
        return;
    }
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while let Some(hit) = lower[i..].find(needle) {
        let at = i + hit;
        out.push_str(&text[i..at]);
        out.push_str(plain);
        i = at + needle.len();
    }
    out.push_str(&text[i..]);
    *text = out;
}

/// An optional field the model filled in with a way of saying "nothing" —
/// scrubbed like the rest when it does carry a sentence.
fn blank_to_none(field: &mut Option<String>) {
    if let Some(value) = field.as_mut() {
        scrub(value);
    }
    let Some(value) = field.as_ref() else { return };
    let v = value.trim().trim_end_matches('.').to_ascii_lowercase();
    if matches!(v.as_str(), "" | "none" | "n/a" | "na" | "nothing" | "null") {
        *field = None;
    } else if value.trim() != value {
        *field = Some(value.trim().to_string());
    }
}

/// A sibling judge's verdict, as the reporter sees it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SiblingVerdict {
    pub judge_name: String,
    pub judge_slug: String,
    pub task_ordinal: i32,
    pub task_title: String,
    pub point_delta: i32,
    pub feedback: String,
    /// The criteria this verdict scored. The reporter is asked for a
    /// per-criterion summary of the session, which it can only write if it
    /// sees the sheet — the prose feedback alone names criteria unevenly and
    /// never by key.
    #[serde(default)]
    pub criteria: Vec<SiblingCriterion>,
}

/// One row of a verdict's criteria sheet, as the reporter is shown it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct SiblingCriterion {
    pub key: String,
    pub score: Option<f64>,
    pub rationale: String,
}

/// How much of one sibling's feedback the reporter is shown. Long enough to
/// carry the reasoning, short enough that twenty of them still leave room for
/// the session evidence.
const SIBLING_FEEDBACK_CAP: usize = 1200;

/// Ceiling on how many sibling verdicts are quoted, newest tasks first.
const SIBLING_VERDICT_MAX: usize = 24;

/// True when this judge writes reports rather than verdicts.
///
/// It is declared, not inferred: `kind: report` in the judge's front matter.
/// An earlier version keyed on a zero-width `rating_scale`, which the seeder
/// rejects outright (`min` must be `< max`, because the scale is real
/// arithmetic everywhere else) — and a magic value would have been a worse
/// declaration than a named one anyway.
pub fn is_report_judge(judge_row: &JudgeRow) -> bool {
    judge_row.kind == super::JUDGE_KIND_REPORT
}

/// Every *other* judge's scored verdict for this (session, player), ordered by
/// task so the reporter reads them in the order the player lived them.
///
/// No judge has ever seen a sibling's verdict before: the existing prior-
/// verdict loaders filter to the same judge id. Summarising the panel is the
/// one job that genuinely needs the cross-judge view, so it is loaded here
/// rather than widened for everyone.
pub async fn load_sibling_verdicts(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    exclude_judge_id: Uuid,
) -> Result<Vec<SiblingVerdict>, JudgeError> {
    let rows = judge_results::Entity::find()
        .filter(judge_results::Column::SessionIdFk.eq(session_id))
        .filter(judge_results::Column::PlayerIdFk.eq(player_id))
        .filter(judge_results::Column::Status.eq(crate::session_completion::JUDGE_RESULT_SCORED))
        .all(db)
        .await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    // Resolve each result's (task, judge) through its task_judges row.
    let tj_ids: Vec<Uuid> = rows.iter().map(|r| r.task_judge_id).collect();
    let tjs: HashMap<Uuid, task_judges::Model> = task_judges::Entity::find()
        .filter(task_judges::Column::Id.is_in(tj_ids))
        .all(db)
        .await?
        .into_iter()
        .map(|t| (t.id, t))
        .collect();

    let judge_ids: Vec<Uuid> = tjs.values().map(|t| t.judge_id).collect();
    let judge_by_id: HashMap<Uuid, judges::Model> = judges::Entity::find()
        .filter(judges::Column::Id.is_in(judge_ids))
        .all(db)
        .await?
        .into_iter()
        .map(|j| (j.id, j))
        .collect();

    let task_ids: Vec<Uuid> = tjs.values().map(|t| t.task_id).collect();
    let task_by_id: HashMap<Uuid, tasks::Model> = tasks::Entity::find()
        .filter(tasks::Column::Id.is_in(task_ids))
        .order_by_asc(tasks::Column::Ordinal)
        .all(db)
        .await?
        .into_iter()
        .map(|t| (t.id, t))
        .collect();

    let mut out: Vec<SiblingVerdict> = rows
        .into_iter()
        .filter_map(|r| {
            let tj = tjs.get(&r.task_judge_id)?;
            if tj.judge_id == exclude_judge_id {
                return None;
            }
            let judge = judge_by_id.get(&tj.judge_id)?;
            let task = task_by_id.get(&tj.task_id)?;
            let criteria = r
                .rating
                .get("criteria")
                .and_then(|v| v.as_array())
                .map(|rows| {
                    rows.iter()
                        .filter_map(|row| {
                            let key = row["key"].as_str()?.trim().to_string();
                            if key.is_empty() {
                                return None;
                            }
                            Some(SiblingCriterion {
                                key,
                                score: row["score"].as_f64(),
                                rationale: row["rationale"].as_str().unwrap_or("").to_string(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let mut feedback = r.feedback;
            if feedback.len() > SIBLING_FEEDBACK_CAP {
                feedback.truncate(SIBLING_FEEDBACK_CAP);
                feedback.push('…');
            }
            Some(SiblingVerdict {
                judge_name: judge.name.clone(),
                judge_slug: judge.slug.clone(),
                task_ordinal: task.ordinal,
                task_title: task.title.clone(),
                point_delta: r.point_delta,
                feedback,
                criteria,
            })
        })
        .collect();

    out.sort_by(|a, b| {
        a.task_ordinal
            .cmp(&b.task_ordinal)
            .then_with(|| a.judge_slug.cmp(&b.judge_slug))
    });
    out.truncate(SIBLING_VERDICT_MAX);
    Ok(out)
}

/// Write the player's session report.
///
/// One model call, one prose answer, `point_delta = 0` on every attached pair
/// so the settle poll clears without the report ever touching a score.
#[allow(clippy::too_many_arguments)]
pub async fn run_session_report(
    db: &DatabaseConnection,
    judge_llm: &dyn super::JudgeLlm,
    repo_dir: &Path,
    session_id: Uuid,
    player_id: Uuid,
    reached_tasks: &[Uuid],
    task_judges: &HashMap<Uuid, TaskJudgeRow>,
    judge_row: &JudgeRow,
    judge_id: Uuid,
    model: &str,
    provider: &str,
) -> Result<SessionJudgeOutput, JudgeError> {
    let started = std::time::Instant::now();

    let scope = crate::judging::tools::ToolScope::from_json(judge_row.ignore_paths.as_deref());
    let dossier =
        build_session_dossier(db, repo_dir, session_id, player_id, reached_tasks, &scope).await?;
    let dossier_json = dossier.to_json();
    let siblings = load_sibling_verdicts(db, session_id, player_id, judge_id).await?;

    let user = build_report_prompt(&dossier_json, &siblings);
    let resp = judge_llm
        .run_agent(&judge_row.prompt, &user, Vec::new(), None)
        .await?;
    let markdown = match resp {
        AgentResponse::Final { text } => text.trim().to_string(),
        // Tools were not offered; a tool call is a protocol violation.
        AgentResponse::ToolCall { name, .. } => {
            return Err(JudgeError::Llm(format!(
                "session report judge received an unexpected tool call '{name}'"
            )));
        }
    };
    if markdown.is_empty() {
        return Err(JudgeError::Llm(
            "session report came back empty".to_string(),
        ));
    }
    // Store canonical JSON when the model answered with the document, so the
    // page never has to strip a fence or a preamble. A model that answered in
    // prose anyway keeps its prose: the page falls back to rendering it, and a
    // readable report beats a dropped one.
    let stored = match parse_report_doc(&markdown) {
        Some(doc) => serde_json::to_string(&doc).unwrap_or(markdown),
        None => {
            tracing::warn!(
                session_id = %session_id,
                player_id = %player_id,
                "session report: model did not answer with the report document; storing its prose"
            );
            markdown
        }
    };
    // `judge_results.feedback` is capped. A report that overruns is trimmed
    // rather than lost: the player would rather read most of it than none.
    let markdown = trim_to_feedback_limit(stored);
    validate_feedback(&markdown).map_err(|_| JudgeError::FeedbackTooLong)?;

    let duration_ms = started.elapsed().as_millis() as i64;
    let mut verdicts = Vec::with_capacity(task_judges.len());
    for tj in task_judges.values() {
        persist_scored_verdict(
            db,
            session_id,
            player_id,
            tj.id,
            0.0,
            0,
            &markdown,
            "",
            model,
            provider,
            duration_ms,
        )
        .await?;
        verdicts.push(TaskVerdict {
            task_id: tj.task_id,
            task_judge_id: tj.id,
            point_delta: Some(0),
            feedback: markdown.clone(),
        });
    }

    let scored = verdicts.len();
    Ok(SessionJudgeOutput {
        verdicts,
        scored,
        failed: 0,
        skipped: 0,
        duration_ms,
        dossier_json,
    })
}

/// Trim a report to what `judge_results.feedback` accepts, on a line boundary
/// where possible so the text does not end mid-word.
fn trim_to_feedback_limit(markdown: String) -> String {
    const LIMIT: usize = crate::validation::judge_results::MAX_FEEDBACK_LEN;
    if markdown.len() <= LIMIT {
        return markdown;
    }
    let mut cut = LIMIT.saturating_sub(64);
    while cut > 0 && !markdown.is_char_boundary(cut) {
        cut -= 1;
    }
    let head = &markdown[..cut];
    let head = match head.rfind('\n') {
        Some(nl) if nl > cut / 2 => &head[..nl],
        _ => head,
    };
    format!("{head}\n\n_(report truncated)_")
}

/// The user prompt: the session's facts, then the panel's verdicts.
fn build_report_prompt(dossier_json: &str, siblings: &[SiblingVerdict]) -> String {
    let mut out = String::with_capacity(dossier_json.len() + 2048);
    out.push_str(
        "## Session evidence (server-collected)\n\n\
         The complete factual record of this player's session: what the repository \
         looked like at the root commit, and per task, the snapshot commit with its \
         diff stat, the probe outcomes, the points banked, and the agent activity \
         reported for that task's work window.\n\n",
    );
    out.push_str("```json\n");
    out.push_str(dossier_json);
    out.push_str("\n```\n\n");

    out.push_str("## What the panel said\n\n");
    if siblings.is_empty() {
        out.push_str(
            "No other judge scored this session — either none were attached, or none \
             had finished when this report was written. Say nothing about the panel; \
             write the rest of the report from the evidence above.\n\n",
        );
    } else {
        out.push_str(
            "Every other judge's verdict for this player, in task order. `point_delta` \
             is what the verdict moved the score by.\n\n",
        );
        // The dossier's own numbering, not a 1-based restatement of it: the
        // report's `ordinal` fields are matched back onto tasks by the page,
        // and a prompt that counts tasks differently from the evidence teaches
        // the model to write the other number.
        for v in siblings {
            out.push_str(&format!(
                "### {} — task {} “{}” ({:+} pts)\n\n{}\n\n",
                v.judge_name, v.task_ordinal, v.task_title, v.point_delta, v.feedback
            ));
            // The sheet, with its keys: `criteria` in the report is keyed back
            // onto these, so the model has to see the spelling it must answer in.
            if !v.criteria.is_empty() {
                out.push_str("Criteria scored (key — score/10 — why):\n\n");
                for c in &v.criteria {
                    let score = c
                        .score
                        .map(|s| format!("{s:.1}"))
                        .unwrap_or_else(|| "not scored".to_string());
                    out.push_str(&format!("- `{}` — {} — {}\n", c.key, score, c.rationale));
                }
                out.push('\n');
            }
        }
    }

    out.push_str(
        "## Write the report\n\n\
         Address the player as “you”. Markdown, no front matter, no headings above \
         level two. Ground every claim in the evidence above — name the task, the \
         probe, or the judge you are drawing from. Do not invent a struggle the \
         record does not show, and do not congratulate a session that went badly.\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn judge(min: f64, max: f64) -> JudgeRow {
        JudgeRow {
            slug: "general".into(),
            name: "General".into(),
            prompt: String::new(),
            rating_scale: serde_json::json!({ "min": min, "max": max, "step": 1 }),
            kind: "llm".into(),
            scope: "session".into(),
            llm_provider_id: None,
            llm_model: None,
            llm_pool_id: None,
            llm_source_order: "pool_first".into(),
            evidence_mode: "tools".into(),
            criteria: None,
            evidence_needs: None,
            max_interactive: None,
            ignore_paths: None,
        }
    }

    #[test]
    fn the_report_kind_declares_a_reporter() {
        let mut j = judge(0.0, 10.0);
        j.kind = super::super::JUDGE_KIND_REPORT.into();
        assert!(is_report_judge(&j));
    }

    #[test]
    fn a_judge_that_can_move_a_score_is_not_a_reporter() {
        assert!(!is_report_judge(&judge(0.0, 10.0)));
        assert!(!is_report_judge(&judge(-1000.0, 0.0)));
    }

    #[test]
    fn the_panel_section_says_so_when_no_one_else_scored() {
        let prompt = build_report_prompt("{}", &[]);
        assert!(prompt.contains("No other judge scored this session"));
        assert!(!prompt.contains("point_delta` is what"));
    }

    #[test]
    fn every_sibling_verdict_reaches_the_prompt() {
        let siblings = vec![
            SiblingVerdict {
                judge_name: "Architecture".into(),
                judge_slug: "architecture".into(),
                task_ordinal: 0,
                task_title: "Bind to a port".into(),
                point_delta: 12,
                feedback: "The accept loop and the protocol are one function.".into(),
                criteria: vec![SiblingCriterion {
                    key: "architecture".into(),
                    score: Some(4.0),
                    rationale: "One function does both jobs.".into(),
                }],
            },
            SiblingVerdict {
                judge_name: "Tests".into(),
                judge_slug: "test-quality".into(),
                task_ordinal: 1,
                task_title: "Concurrent clients".into(),
                point_delta: -3,
                feedback: "No test opens a socket.".into(),
                criteria: Vec::new(),
            },
        ];
        let prompt = build_report_prompt("{}", &siblings);
        // Task numbers as the evidence states them — the page shows the same.
        assert!(prompt.contains("Architecture — task 0 “Bind to a port” (+12 pts)"));
        assert!(prompt.contains("Tests — task 1 “Concurrent clients” (-3 pts)"));
        assert!(prompt.contains("No test opens a socket."));
        // The sheet rides along with its keys: the report answers by key, so
        // the model has to be shown the spelling it must use.
        assert!(prompt.contains("`architecture` — 4.0 — One function does both jobs."));
    }

    #[test]
    fn an_overlong_report_is_trimmed_rather_than_rejected() {
        let long = "a line of the report\n".repeat(2000);
        assert!(long.len() > crate::validation::judge_results::MAX_FEEDBACK_LEN);
        let trimmed = trim_to_feedback_limit(long);
        assert!(validate_feedback(&trimmed).is_ok());
        assert!(trimmed.ends_with("_(report truncated)_"));
    }

    const DOC: &str = r#"{
      "built": {"brief": "A SQL engine that filters and sorts.",
                "tasks": [{"ordinal": 1, "note": "projection"}]},
      "friction": [{"ordinal": 3, "what_happened": "the checks failed twice", "why": null}],
      "judges": [{"judge": "Architecture", "good": "clear split", "improve": "SELECT is large"}],
      "improve": ["write a test per operator"]
    }"#;

    #[test]
    fn the_report_document_parses() {
        let doc = parse_report_doc(DOC).expect("parses");
        assert_eq!(doc.built.tasks.len(), 1);
        assert_eq!(doc.friction[0].ordinal, 3);
        assert!(doc.friction[0].why.is_none());
        assert_eq!(doc.judges[0].improve.as_deref(), Some("SELECT is large"));
        assert_eq!(doc.improve.len(), 1);
    }

    #[test]
    fn the_per_criterion_summary_survives_and_is_pruned() {
        // The scorecard shows a criterion once for the session; this is the
        // description under it. An entry the page cannot place — no key, or
        // nothing said — would render as a criterion with no words at all, so
        // it is dropped and the page falls back to the panel's own note.
        let raw = r#"{
          "built": {"brief": "A server.", "tasks": []},
          "criteria": [
            {"key": "architecture", "summary": "Held at 9 across all three tasks."},
            {"key": "tests", "summary": "   "},
            {"key": "  ", "summary": "orphaned"},
            {"key": "data", "summary": "Rose once the probe stopped failing."}
          ]
        }"#;
        let doc = parse_report_doc(raw).expect("parses");
        let keys: Vec<&str> = doc.criteria.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, ["architecture", "data"]);
        // Our vocabulary is scrubbed here as it is everywhere else.
        assert_eq!(
            doc.criteria[1].summary,
            "Rose once the check stopped failing."
        );
    }

    #[test]
    fn a_report_without_criteria_still_parses() {
        // Every report written before the field existed.
        let doc = parse_report_doc(DOC).expect("parses");
        assert!(doc.criteria.is_empty());
    }

    #[test]
    fn a_fenced_or_prefaced_answer_still_parses() {
        // The two things models do to JSON.
        let fenced = format!("```json\n{DOC}\n```");
        assert!(parse_report_doc(&fenced).is_some());
        let prefaced = format!("Here is your report:\n\n{DOC}");
        assert!(parse_report_doc(&prefaced).is_some());
    }

    #[test]
    fn prose_is_not_mistaken_for_a_document() {
        // The fallback path: the page renders it as text rather than dropping it.
        assert!(parse_report_doc("## What you built\n\nA server.").is_none());
        assert!(parse_report_doc("{\"built\": {\"brief\": \"  \"}}").is_none());
        assert!(parse_report_doc("").is_none());
    }

    #[test]
    fn our_word_for_a_check_never_reaches_the_player() {
        // gpt-oss-120b wrote "probe" nine times in one report despite the
        // prompt forbidding it. The prompt still asks; this makes it true.
        let raw = r#"{
          "built": {"brief": "A probe-driven engine.", "tasks": [{"ordinal": 1, "note": "the probe passed"}]},
          "friction": [{"ordinal": 3, "what_happened": "Probes failed twice", "why": "the probe expected ints"}],
          "judges": [{"judge": "Tests", "good": "probe coverage", "improve": "add probes"}],
          "improve": ["run the probes locally first"]
        }"#;
        let doc = parse_report_doc(raw).expect("parses");
        let all = serde_json::to_string(&doc).expect("serialize");
        assert!(!all.to_lowercase().contains("probe"), "{all}");
        assert_eq!(doc.built.brief, "A check-driven engine.");
        assert_eq!(doc.friction[0].what_happened, "Checks failed twice");
    }

    #[test]
    fn the_evidence_flag_names_never_reach_the_player() {
        // Prod, 2026-08-22: three of the four Money Tracker debriefs explained
        // a task in our schema's words — "the task is flagged with
        // no_task_commit and no_agent_stats", "commit_sha is null". A player
        // reads that as an error message about themselves.
        let raw = r#"{
          "built": {"brief": "A tracker.", "tasks": [{"ordinal": 0, "note": "empty_task_commit"}]},
          "friction": [{
            "ordinal": 1,
            "what_happened": "you passed with passed_without_changes",
            "why": "commit_sha is null and the task is flagged with no_task_commit and no_agent_stats"
          }],
          "judges": [{"judge": "Correctness", "good": "g", "improve": "zero_agent_activity"}],
          "improve": ["check the point_delta"]
        }"#;
        let doc = parse_report_doc(raw).expect("parses");
        let all = serde_json::to_string(&doc).expect("serialize");
        for (flag, _) in JARGON {
            assert!(!all.contains(flag), "{flag} survived: {all}");
        }
        assert_eq!(
            doc.friction[0].why.as_deref(),
            Some(
                "there is no commit for the task and the task is flagged with \
                 no commit for that task and no reported agent activity"
            )
        );
    }

    #[test]
    fn a_judge_that_asked_for_nothing_leaves_the_field_empty() {
        // "None" as a string renders as a cell reading None.
        let raw = r#"{
          "built": {"brief": "A server."},
          "judges": [
            {"judge": "A", "good": "g", "improve": "None"},
            {"judge": "B", "good": "g", "improve": "n/a"},
            {"judge": "C", "good": "g", "improve": "  Split the parser.  "}
          ],
          "friction": [{"ordinal": 1, "what_happened": "w", "why": "nothing"}],
          "improve": ["a real change", "   "]
        }"#;
        let doc = parse_report_doc(raw).expect("parses");
        assert!(doc.judges[0].improve.is_none());
        assert!(doc.judges[1].improve.is_none());
        assert_eq!(doc.judges[2].improve.as_deref(), Some("Split the parser."));
        assert!(doc.friction[0].why.is_none());
        assert_eq!(doc.improve, vec!["a real change".to_string()]);
    }

    #[test]
    fn only_the_worst_rough_patches_survive() {
        // The model filed eight, in severity order. Three is the contract.
        let items: Vec<String> = (1..=8)
            .map(|i| format!(r#"{{"ordinal": {i}, "what_happened": "w{i}", "why": null}}"#))
            .collect();
        let raw = format!(
            r#"{{"built": {{"brief": "A server."}}, "friction": [{}]}}"#,
            items.join(",")
        );
        let doc = parse_report_doc(&raw).expect("parses");
        assert_eq!(doc.friction.len(), 3);
        assert_eq!(
            doc.friction[0].ordinal, 1,
            "the model's own ordering is kept"
        );
    }

    #[test]
    fn a_report_that_fits_is_left_alone() {
        let short = "## What you built\n\nA server.".to_string();
        assert_eq!(trim_to_feedback_limit(short.clone()), short);
    }
}
