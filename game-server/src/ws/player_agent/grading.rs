use crate::state::GameServerState;
use crate::ws::player_agent::scoring::{NoResponseInput, record_no_response};
use arena_core::entities::{activity_event, players, probes, tasks, tests};
use arena_core::memory::memory_placeholder_names;
use arena_core::probe_engine::{
    FixtureSample, ProbeEngineError, SecretMeta, eval_js_fixtures_with_meta,
    evaluate_answer_with_memory, js_fixture_scalars, normalize_brace_placeholders,
    parse_js_fixture_script, render_command_shell_aware, render_command_with_memory,
    sample_fixtures,
};
use arena_core::protocol::{
    ArenaFrame, PROBE_DECLINED_STDOUT, PlayerAgentClientFrame, PlayerAgentFrame, ProbeOutcome,
    ZmqEvent,
};
use arena_core::task_template::FixtureDef;
use axum::extract::ws::{Message, WebSocket};
use chrono::Utc;
use sea_orm::prelude::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde_json::Map;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::broadcast;
use uuid::Uuid;

pub struct ResolvedFixtures {
    pub fixture_defs: Vec<FixtureDef>,
    pub fixture_scalars: HashMap<String, String>,
    pub rendered_command: String,
    pub fixture_values_for_eval: HashMap<String, FixtureSample>,
    pub js_validation_mode: bool,
    pub js_fixtures_for_eval: Option<Map<String, serde_json::Value>>,
    pub secret_meta: SecretMeta,
    pub expected_answer_display: Option<String>,
}

/// Resolve the fixtures, command, and secret metadata for one probe of
/// `adapted_test`. Returns `None` when an unrecoverable error means the
/// caller should break out of the dispatch loop (disconnect).
///
/// `memory` is the merged per-player session-memory map (project defaults ⊕
/// extracted values); it is bound as a nested `memory` object so templates
/// can reference `{memory.<key>}`. Values are player-authored content, so
/// they are shell-quoted for the command: the JS branch quotes per placeholder
/// position (see [`render_command_shell_aware`]), the legacy branch pre-quotes
/// with shlex.
pub async fn resolve_test_fixtures(
    adapted_test: &tests::Model,
    session_id: Uuid,
    memory: &std::collections::BTreeMap<String, String>,
) -> Option<ResolvedFixtures> {
    // Shell-quote memory values once for command rendering. A value that
    // cannot be quoted (embedded NUL) is dropped with a warning rather than
    // aborting the probe.
    let mut memory_quoted = std::collections::BTreeMap::new();
    for (k, v) in memory {
        match shlex::try_quote(v) {
            Ok(q) => {
                memory_quoted.insert(k.clone(), q.into_owned());
            }
            Err(_) => {
                tracing::warn!(session_id = %session_id, key = %k, "player_agent: memory value quote failed, dropping");
            }
        }
    }
    let memory_names = memory_placeholder_names(memory);
    let mut js_validation_mode = false;
    let mut js_fixtures_for_eval: Option<Map<String, serde_json::Value>> = None;
    // Secret metadata extracted from the `js fixtures` block (via
    // `__secret` / `__secret_expected`). For non-JS fixtures it stays
    // empty (no redaction). Persisted on the probe row so the REST API
    // can redact from the same source of truth.
    let mut secret_meta = SecretMeta::default();

    let (fixture_defs, fixture_scalars, rendered_command, fixture_values_for_eval) = if let Some(
        js_script,
    ) =
        parse_js_fixture_script(&adapted_test.fixture_definitions)
    {
        let outcome = match eval_js_fixtures_with_meta(&js_script) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(session_id = %session_id, error = %e, "player_agent: js fixtures eval failed");
                return None;
            }
        };
        secret_meta.fixtures = outcome.secret_keys;
        secret_meta.expected = outcome.secret_expected;
        let js_fixtures = outcome.fixtures;
        let raw_scalars = js_fixture_scalars(&js_fixtures);

        let mut known_names: Vec<String> = raw_scalars.keys().cloned().collect();
        known_names.extend(memory_names.iter().cloned());
        let normalized_tpl =
            normalize_brace_placeholders(&adapted_test.command_template, &known_names);
        // Raw values in, quoting decided per placeholder position: a value
        // inside the template's own `"{list}"` must NOT be shlex-quoted on
        // top (the player's program received literal quotes around every
        // comma-carrying list), while a bare `R={memory.run}` must be.
        let rendered = match render_command_shell_aware(&normalized_tpl, &raw_scalars, memory) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(session_id = %session_id, error = %e, "player_agent: render js command failed");
                return None;
            }
        };

        js_validation_mode = !adapted_test.answer_template.trim().is_empty();
        js_fixtures_for_eval = Some(js_fixtures);
        (
            Vec::<FixtureDef>::new(),
            raw_scalars,
            rendered,
            HashMap::new(),
        )
    } else {
        let fixture_defs: Vec<FixtureDef> = match serde_json::from_str(
            &adapted_test.fixture_definitions,
        ) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(session_id = %session_id, error = %e, "player_agent: bad fixture_definitions JSON");
                return None;
            }
        };

        let fixture_result = {
            let mut rng = rand::thread_rng();
            sample_fixtures(&fixture_defs, &mut rng)
        };
        let fixture_values = match fixture_result {
            Ok(v) => v,
            Err(ProbeEngineError::EmptyPool(name)) => {
                tracing::warn!(session_id = %session_id, var = %name, "player_agent: empty fixture pool");
                return None;
            }
            Err(e) => {
                tracing::warn!(session_id = %session_id, error = %e, "player_agent: fixture sampling failed");
                return None;
            }
        };

        let fixture_scalars: HashMap<String, String> = fixture_values
            .iter()
            .map(|(name, s)| {
                (
                    name.clone(),
                    s.key.as_deref().unwrap_or(&s.value).to_string(),
                )
            })
            .collect();

        // This branch renders `{{name}}` templates directly; normalize only
        // the dotted memory tokens so `{memory.key}` works here too.
        let tpl = normalize_brace_placeholders(&adapted_test.command_template, &memory_names);
        let rendered_command = match render_command_with_memory(
            &tpl,
            &fixture_defs,
            &fixture_values,
            &memory_quoted,
        ) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(session_id = %session_id, error = %e, "player_agent: render_command failed");
                return None;
            }
        };
        (
            fixture_defs,
            fixture_scalars,
            rendered_command,
            fixture_values,
        )
    };

    let expected_answer_display: Option<String> = if !adapted_test.answer_template.trim().is_empty()
    {
        Some(adapted_test.answer_template.clone())
    } else {
        None
    };

    Some(ResolvedFixtures {
        fixture_defs,
        fixture_scalars,
        rendered_command,
        fixture_values_for_eval,
        js_validation_mode,
        js_fixtures_for_eval,
        secret_meta,
        expected_answer_display,
    })
}

/// Grade the client's `TestResult` (or timeout / stale frame) for one probe.
/// Mutates `current_interval_secs` and `dispatch_next_immediately` as the
/// original inline logic did. Returns `true` when the caller must break out
/// of the dispatch loop.
#[allow(clippy::too_many_arguments)]
/// Parse and persist a `report: todo` probe's measurement. Returns a
/// `SnapshotRequest{todo_progress}` frame to send when the plan moved:
/// it just appeared (first report with items — an all-unchecked fresh
/// TODO.md must reach the repo, the judges pick their artifacts from it),
/// a checked count grew, or the item list was restructured. `None`
/// otherwise. The TODO content itself is untrusted display text — parsing
/// caps it, the frontend escapes it.
pub async fn handle_todo_report(
    state: &GameServerState,
    adapted_test: &tests::Model,
    task_row: &tasks::Model,
    player_id: Uuid,
    probe_id: Uuid,
    stdout_trimmed: &str,
) -> Option<PlayerAgentFrame> {
    let config = adapted_test.probe_config.as_ref()?;
    let config = arena_core::evaluation::ProbeConfig::from_json(config).ok()?;
    if config.report != Some(arena_core::evaluation::ReportKind::Todo) {
        return None;
    }
    let report = arena_core::evaluation::parse_todo_report(stdout_trimmed);

    // The previous report, for progress detection — the latest earlier probe
    // of this test that carried one.
    let previous: Option<(u32, u32)> = probes::Entity::find()
        .filter(probes::Column::TestId.eq(adapted_test.id))
        .filter(probes::Column::PlayerId.eq(player_id))
        .filter(probes::Column::Id.ne(probe_id))
        .filter(probes::Column::ResultJson.is_not_null())
        .order_by_desc(probes::Column::DispatchedAt)
        .one(&state.db)
        .await
        .ok()
        .flatten()
        .and_then(|p| p.result_json)
        .and_then(|v| {
            Some((
                v["todo"]["checked"].as_u64()? as u32,
                v["todo"]["total"].as_u64()? as u32,
            ))
        });

    let progressed = match previous {
        // First sighting of a plan: even all-unchecked, it belongs in the
        // repo now — judges read it to decide what artifacts to request.
        None => true,
        Some((prev_checked, prev_total)) => {
            report.checked > prev_checked || report.total != prev_total
        }
    };
    let has_items = report.total > 0;
    let result_json = serde_json::json!({ "todo": report });
    let _ = probes::Entity::update_many()
        .col_expr(probes::Column::ResultJson, Expr::value(result_json))
        .filter(probes::Column::Id.eq(probe_id))
        .exec(&state.db)
        .await;

    (progressed && has_items).then(|| PlayerAgentFrame::SnapshotRequest {
        task_id: task_row.id,
        task_title: task_row.title.clone(),
        reason: arena_core::protocol::SNAPSHOT_REASON_TODO_PROGRESS.to_string(),
    })
}

/// Deterministic `capture_memory` probes: on a passing run, store the probe's
/// trimmed stdout into session memory under the configured key. This is how a
/// task feeds a deterministic value (e.g. the `run:` command a setup probe
/// prints) into `{memory.<key>}` without an LLM extraction or a pushed
/// snapshot — so it works the same in TUI and headless play.
pub async fn handle_capture_memory(
    state: &GameServerState,
    adapted_test: &tests::Model,
    session_id: Uuid,
    player_id: Uuid,
    pass: bool,
    stdout_trimmed: &str,
) {
    if !pass {
        return;
    }
    let Some(config) = adapted_test.probe_config.as_ref() else {
        return;
    };
    let Ok(config) = arena_core::evaluation::ProbeConfig::from_json(config) else {
        return;
    };
    let Some(key) = config.capture_memory else {
        return;
    };
    crate::session_memory::capture_value(state, session_id, player_id, &key, stdout_trimmed).await;
}

/// The opt-in point table of a config-carrying probe: `Some(points)` for any
/// probe with a `probe_config` (defaulting to 0/0), `None` for legacy probes.
pub fn probe_points(adapted_test: &tests::Model) -> Option<arena_core::evaluation::ProbePoints> {
    let config = adapted_test.probe_config.as_ref()?;
    match arena_core::evaluation::ProbeConfig::from_json(config) {
        // A capture-only probe (deterministic, captures stdout into memory,
        // no explicit `points`) is still an ordinary task check — it must
        // score off the task pay table, not the 0/0 measurement default that
        // an open-ended config implies. Only a config that opts into `points`
        // overrides the pay table.
        Ok(cfg) if cfg.capture_memory.is_some() && cfg.points.is_none() => None,
        Ok(cfg) => Some(cfg.points.unwrap_or_default()),
        Err(e) => {
            tracing::warn!(test_id = %adapted_test.id, error = %e, "unparseable probe_config; scoring as measurement (0/0)");
            Some(arena_core::evaluation::ProbePoints::default())
        }
    }
}

/// Why an agent's `TestResult` carries no measurement, if it carries none.
///
/// A probe the agent never ran has nothing to grade: the frame holds a
/// placeholder — a decline notice, or an empty string — instead of output.
/// Handing that placeholder to the validation script is how a declined probe
/// used to *pass*: `[ololo] probe command declined by player` satisfies a
/// predicate like `result.trim().length > 0`, and the player collected full
/// points for refusing to run the command. Whatever this returns is a hard
/// fail, whatever the script would have said.
fn unrunnable_reason(error: Option<&str>, exit_code: Option<i32>, stdout: &str) -> Option<String> {
    if let Some(reason) = error.map(str::trim).filter(|r| !r.is_empty()) {
        return Some(reason.to_string());
    }
    // Agents older than the `error` field announce a decline only through the
    // stdout placeholder.
    if stdout.trim() == PROBE_DECLINED_STDOUT {
        return Some("declined by player".to_string());
    }
    // The protocol defines `exit_code: None` as "the process could not be
    // started", which no real measurement ever reports. A command killed by a
    // signal comes back as `Some(-1)`, so this cannot swallow one.
    if exit_code.is_none() {
        return Some("probe command could not be executed".to_string());
    }
    None
}

/// Grade a probe's stdout against the section's validation — the three
/// grading modes (JS validation / minijinja template / direct equality) in
/// one place. Shared by the live loop, the execution judge, and server-side
/// probes so a solution can never pass one grader and fail another.
///
/// Returns `(pass, display_expected, display_actual)` — the display values
/// are set when the JS validation called `assertEqual`.
#[allow(clippy::too_many_arguments)]
pub fn grade_stdout(
    answer_template: &str,
    js_validation_mode: bool,
    js_fixtures_for_eval: &Option<Map<String, serde_json::Value>>,
    fixture_defs: &[FixtureDef],
    fixture_values_for_eval: &HashMap<String, FixtureSample>,
    expected_answer_display: &Option<String>,
    memory: &std::collections::BTreeMap<String, String>,
    stdout_trimmed: &str,
    exit_code: Option<i64>,
) -> (bool, Option<String>, Option<String>) {
    if js_validation_mode {
        match js_fixtures_for_eval {
            Some(fx) => {
                let outcome = arena_core::probe_engine::eval_js_validation_outcome_full(
                    answer_template,
                    fx,
                    memory,
                    stdout_trimmed,
                    exit_code,
                )
                .unwrap_or_default();
                (outcome.pass, outcome.expected, outcome.actual)
            }
            None => (false, None, None),
        }
    } else if !answer_template.trim().is_empty() {
        let p = evaluate_answer_with_memory(
            answer_template,
            fixture_defs,
            fixture_values_for_eval,
            memory,
            stdout_trimmed,
        )
        .unwrap_or(false);
        (p, None, None)
    } else {
        (
            expected_answer_display.as_deref() == Some(stdout_trimmed),
            None,
            None,
        )
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn grade_test_result(
    result: Result<Option<PlayerAgentClientFrame>, tokio::time::error::Elapsed>,
    state: &GameServerState,
    socket: &mut WebSocket,
    session_id: Uuid,
    player_id: Uuid,
    join_code: &str,
    probe_id: Uuid,
    task_id: Uuid,
    task_row: &tasks::Model,
    adapted_test: &tests::Model,
    fixture_defs: &[FixtureDef],
    fixture_values_for_eval: &HashMap<String, FixtureSample>,
    expected_answer_display: &Option<String>,
    secret_meta: &SecretMeta,
    js_validation_mode: bool,
    js_fixtures_for_eval: &Option<Map<String, serde_json::Value>>,
    memory: &std::collections::BTreeMap<String, String>,
    interval_increment_secs: i32,
    min_interval_secs: i32,
    max_interval_secs: i32,
    current_interval_secs: &mut i32,
    dispatch_next_immediately: &mut bool,
) -> bool {
    match result {
        Ok(Some(PlayerAgentClientFrame::TestResult {
            probe_id: recv_id,
            stdout,
            exit_code,
            duration_ms,
            error,
        })) if recv_id == probe_id => {
            let stdout_trimmed = stdout.trim_end().to_string();
            // A probe that never ran is failed here, before any validation
            // sees the placeholder standing in for its output.
            let unrunnable = unrunnable_reason(error.as_deref(), exit_code, &stdout_trimmed);
            // When the validation script calls `assertEqual(actual, expected)`,
            // the engine stashes display values for both operands. These
            // override the raw answer_template (expected) and stdout (actual)
            // in the ProbeGraded frame so the TUI shows real values (e.g.
            // "42" instead of "Number(result.trim()) === n1 * n2"). The DB
            // `expected_answer` is also updated so the frontend probe panel
            // shows the computed expected value. `output` keeps raw stdout.
            let (pass, display_expected, display_actual) = match &unrunnable {
                Some(reason) => (false, None, Some(format!("[ololo] {reason}"))),
                None => grade_stdout(
                    &adapted_test.answer_template,
                    js_validation_mode,
                    js_fixtures_for_eval,
                    fixture_defs,
                    fixture_values_for_eval,
                    expected_answer_display,
                    memory,
                    &stdout_trimmed,
                    exit_code.map(i64::from),
                ),
            };
            // A measurement probe (extended config, no validation) succeeds
            // by running: it reports data for the judges, it does not grade.
            // Marking it "error" painted every open-ended TODO report red and
            // dragged the task's displayed status to "failed" mid-run. "By
            // running" is the whole condition, so a declined one stays failed.
            let is_measurement = adapted_test.probe_config.is_some()
                && adapted_test.answer_template.trim().is_empty()
                && !js_validation_mode;
            let pass = pass || (is_measurement && unrunnable.is_none());
            let outcome = if pass { "pass" } else { "error" };
            // A probe carrying an extended config scores its own opt-in
            // points (default 0/0 — open-ended probes are measurements, the
            // judges convert them into score). Legacy probes keep the task's
            // pay table.
            let delta = match probe_points(adapted_test) {
                Some(points) => {
                    if pass {
                        points.pass
                    } else {
                        points.fail
                    }
                }
                None => {
                    if pass {
                        task_row.point_value
                    } else {
                        task_row.fail_points
                    }
                }
            };

            let received_at = Utc::now();
            // ProbeGraded frame carries the resolved value for the ololo
            // TUI "expected" field — it does NOT mutate expected_answer.
            let graded_expected: Option<String> = display_expected
                .clone()
                .or_else(|| expected_answer_display.clone());
            let mut probe_update = probes::Entity::update_many()
                .col_expr(probes::Column::Outcome, Expr::value(outcome))
                .col_expr(probes::Column::ResolvedAt, Expr::value(received_at))
                .col_expr(probes::Column::Output, Expr::value(stdout_trimmed.clone()))
                .col_expr(probes::Column::ExitCode, Expr::value(exit_code))
                .col_expr(probes::Column::DurationMs, Expr::value(duration_ms as i64))
                .col_expr(probes::Column::PointDelta, Expr::value(delta))
                .col_expr(probes::Column::UpdatedAt, Expr::value(received_at));
            // Persist the computed expected value into resolved_answer
            // (separate from expected_answer, which holds the raw
            // predicate template for the "Task answer:" panel). The
            // frontend reads resolved_answer for the "Expected:" panel.
            if let Some(ref exp) = display_expected {
                probe_update =
                    probe_update.col_expr(probes::Column::ResolvedAnswer, Expr::value(exp.clone()));
            }
            let _ = probe_update
                .filter(probes::Column::Id.eq(probe_id))
                .filter(probes::Column::Outcome.is_null())
                .exec(&state.db)
                .await;
            crate::session_log_store::record(
                crate::session_log_store::base_dir(),
                session_id,
                Some(player_id),
                "probe_graded",
                serde_json::json!({
                    "player_id": player_id,
                    "task_id": task_id,
                    "task_ordinal": task_row.ordinal,
                    "probe_id": probe_id,
                    "outcome": outcome,
                    "answer": stdout_trimmed,
                    "expected": graded_expected,
                    "point_delta": delta,
                    "duration_ms": duration_ms,
                }),
            )
            .await;

            // Structured report probes: parse the plan out of stdout, store
            // it as the measurement, and — when the plan moved — ask the
            // agent for a wip snapshot so server-side probes see fresh HEAD.
            // A probe that never ran has no plan to parse, and recording its
            // zeroes would become the baseline the next real report is
            // compared against.
            if unrunnable.is_none()
                && let Some(request) = handle_todo_report(
                    state,
                    adapted_test,
                    task_row,
                    player_id,
                    probe_id,
                    &stdout_trimmed,
                )
                .await
            {
                let json = serde_json::to_string(&request).unwrap_or_default();
                let _ = socket.send(Message::Text(json)).await;
            }

            // Deterministic capture probes: a passing run writes its stdout
            // into session memory so later `{memory.<key>}` renders use it —
            // no LLM extraction or pushed snapshot required.
            if unrunnable.is_none() {
                handle_capture_memory(
                    state,
                    adapted_test,
                    session_id,
                    player_id,
                    pass,
                    &stdout_trimmed,
                )
                .await;
            }

            crate::ws::player_agent::scoring::insert_task_result(
                state,
                session_id,
                Some(task_id),
                delta,
                &stdout_trimmed,
                player_id,
            )
            .await;
            crate::ws::player_agent::scoring::publish_score_change(
                state,
                session_id,
                player_id,
                delta as i64,
                join_code,
            )
            .await;
            crate::ws::player_agent::scoring::broadcast_leaderboard(state, session_id, join_code)
                .await;

            // Persist + emit TaskScored for probe-pass activity log events.
            if delta > 0 {
                let now = Utc::now();
                let player_display_name = players::Entity::find_by_id(player_id)
                    .one(&state.db)
                    .await
                    .ok()
                    .flatten()
                    .map(|p| p.display_name)
                    .unwrap_or_default();
                let version = state
                    .session_registry
                    .get(join_code)
                    .and_then(|e| e.cache.read().ok().map(|c| c.version))
                    .unwrap_or(0);

                let activity_row = activity_event::ActiveModel {
                    id: sea_orm::ActiveValue::Set(Uuid::new_v4()),
                    session_id_fk: sea_orm::ActiveValue::Set(session_id),
                    player_id_fk: sea_orm::ActiveValue::Set(player_id),
                    task_id_fk: sea_orm::ActiveValue::Set(task_id),
                    event_kind: sea_orm::ActiveValue::Set("task_scored".to_string()),
                    task_ordinal: sea_orm::ActiveValue::Set(task_row.ordinal),
                    task_title: sea_orm::ActiveValue::Set(task_row.title.clone()),
                    player_display_name: sea_orm::ActiveValue::Set(player_display_name.clone()),
                    judge_name: sea_orm::ActiveValue::Set(None),
                    point_delta: sea_orm::ActiveValue::Set(Some(delta)),
                    timestamp: sea_orm::ActiveValue::Set(now),
                    version: sea_orm::ActiveValue::Set(version as i64),
                    detail: sea_orm::ActiveValue::Set(None),
                };
                if activity_event::Entity::insert(activity_row)
                    .exec(&state.db)
                    .await
                    .is_ok()
                {
                    // Probe-pass scoring, not a judge verdict: judge fields stay
                    // empty so the server bridge skips the player judge frame.
                    let event = ZmqEvent::JudgeScored {
                        join_code: join_code.to_string(),
                        player_id,
                        player_display_name,
                        task_id,
                        task_ordinal: task_row.ordinal,
                        task_title: task_row.title.clone(),
                        point_delta: delta,
                        judge_slug: String::new(),
                        judge_name: String::new(),
                        rating: 0.0,
                        feedback: String::new(),
                        duration_ms: None,
                        timestamp: now,
                        version,
                        detail: None,
                    };
                    state.event_publisher.publish(&event).await;
                }
            }

            let graded = PlayerAgentFrame::ProbeGraded {
                probe_id,
                outcome: if pass {
                    ProbeOutcome::Pass
                } else {
                    ProbeOutcome::Error
                },
                point_delta: delta,
                // Suppress the expected value when the task marked it
                // secret — the player must not see the answer even after
                // grading. The DB row keeps resolved_answer for grading.
                expected: if secret_meta.expected {
                    None
                } else {
                    graded_expected.clone()
                },
                actual: display_actual.clone().or(Some(stdout_trimmed.clone())),
            };
            let graded_json = serde_json::to_string(&graded).unwrap_or_default();
            let _ = socket.send(Message::Text(graded_json)).await;

            if pass {
                *current_interval_secs = min_interval_secs;
                *dispatch_next_immediately = true;
            } else {
                *current_interval_secs = (*current_interval_secs + interval_increment_secs)
                    .min(max_interval_secs)
                    .max(min_interval_secs);
            }
            false
        }
        Ok(Some(_)) => {
            tracing::warn!(session_id = %session_id, "player_agent: received stale/mismatched TestResult");
            record_no_response(
                state,
                socket,
                session_id,
                player_id,
                join_code,
                &NoResponseInput {
                    probe_id,
                    task_id,
                    no_response_points: probe_points(adapted_test)
                        .map(|p| p.fail.min(0))
                        .unwrap_or(task_row.no_response_points),
                    expected_answer_display,
                    secret_expected: secret_meta.expected,
                    interval_increment_secs,
                    min_interval_secs,
                    max_interval_secs,
                },
                current_interval_secs,
            )
            .await;
            false
        }
        Ok(None) => true,
        Err(_timeout) => {
            tracing::debug!(session_id = %session_id, "player_agent: probe deadline exceeded -> no_response");
            record_no_response(
                state,
                socket,
                session_id,
                player_id,
                join_code,
                &NoResponseInput {
                    probe_id,
                    task_id,
                    no_response_points: probe_points(adapted_test)
                        .map(|p| p.fail.min(0))
                        .unwrap_or(task_row.no_response_points),
                    expected_answer_display,
                    secret_expected: secret_meta.expected,
                    interval_increment_secs,
                    min_interval_secs,
                    max_interval_secs,
                },
                current_interval_secs,
            )
            .await;
            if socket.send(Message::Ping(vec![])).await.is_err() {
                return true;
            }
            false
        }
    }
}

/// Wait for `SessionStarted` during the lobby phase, forwarding countdown
/// frames to the client. Returns `true` if the caller should disconnect.
pub async fn await_lobby_started(
    socket: &mut WebSocket,
    session_rx: &mut Option<broadcast::Receiver<ArenaFrame>>,
    session_id: Uuid,
    lobby_timer_secs: u64,
) -> bool {
    tracing::info!(
        session_id = %session_id,
        "player_agent: lobby active — waiting for SessionStarted before probes",
    );
    if let Some(rx) = session_rx.as_mut() {
        let lobby_timeout_secs = lobby_timer_secs + 10;
        let lobby_deadline = tokio::time::sleep(Duration::from_secs(lobby_timeout_secs));
        tokio::pin!(lobby_deadline);
        'lobby_wait: loop {
            tokio::select! {
                _ = &mut lobby_deadline => {
                    tracing::warn!(
                        session_id = %session_id,
                        "player_agent: lobby wait timed out, proceeding anyway",
                    );
                    break 'lobby_wait;
                }
                result = rx.recv() => {
                    match result {
                        Ok(frame) => {
                            // Forward all lobby frames (LobbyCountdown, SessionStarted)
                            // to the client so it can display the countdown and stay alive.
                            if let Ok(json) = serde_json::to_string(&frame)
                                && socket.send(Message::Text(json)).await.is_err() {
                                    tracing::info!(
                                        session_id = %session_id,
                                        "player_agent: client disconnected during lobby",
                                    );
                                    return true;
                                }
                            if matches!(frame, ArenaFrame::SessionStarted { .. }) {
                                tracing::info!(
                                    session_id = %session_id,
                                    "player_agent: SessionStarted received, starting probes",
                                );
                                break 'lobby_wait;
                            }
                        }
                        Err(_) => break 'lobby_wait, // channel closed
                    }
                }
                msg = socket.recv() => {
                    match msg {
                        Some(Ok(Message::Close(_))) | None => {
                            tracing::info!(
                                session_id = %session_id,
                                "player_agent: client closed connection during lobby",
                            );
                            return true;
                        }
                        Some(Ok(Message::Ping(data))) => {
                            let _ = socket.send(Message::Pong(data)).await;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    false
}

/// Outcome of [`drain_during_sleep`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepOutcome {
    /// The sleep ran its course.
    Elapsed,
    /// The client disconnected — the caller should stop driving the socket.
    Disconnected,
    /// The agent announced a pushed completion flag file mid-sleep; the
    /// caller should cut the sleep short and dispatch the completion probe
    /// now instead of letting the flag sit until the next interval tick.
    CompletionFlag,
}

/// Drain the session broadcast channel during the inter-probe sleep so
/// RunningCountdown (and any other session) frames reach the client without
/// waiting for the next probe dispatch. Also handles clean client disconnect
/// and pings.
pub async fn drain_during_sleep(
    socket: &mut WebSocket,
    session_rx: &mut Option<broadcast::Receiver<ArenaFrame>>,
    session_id: Uuid,
    sleep_secs: u64,
) -> SleepOutcome {
    let inter_probe_sleep = tokio::time::sleep(Duration::from_secs(sleep_secs));
    tokio::pin!(inter_probe_sleep);
    loop {
        tokio::select! {
            _ = &mut inter_probe_sleep => break,
            bcast = async {
                if let Some(rx) = session_rx.as_mut() {
                    rx.recv().await.ok()
                } else {
                    std::future::pending::<Option<ArenaFrame>>().await
                }
            } => {
                if let Some(frame) = bcast {
                    // Bridge dashboard frames to the agent wire. Only
                    // `SessionComplete` needs the agent variant (carries a
                    // `reason` ololo shows as Cancelled/Complete); everything
                    // else (RunningCountdown, LobbyCountdown, …) parses as the
                    // agent frame directly.
                    let json = match frame {
                        ArenaFrame::SessionComplete { reason, .. } => {
                            serde_json::to_string(&PlayerAgentFrame::SessionComplete {
                                session_id,
                                reason: Some(reason),
                            })
                            .unwrap_or_default()
                        }
                        ArenaFrame::SessionPaused {
                            seconds_remaining, ..
                        } => serde_json::to_string(&PlayerAgentFrame::SessionPaused {
                            session_id,
                            seconds_remaining,
                        })
                        .unwrap_or_default(),
                        ArenaFrame::SessionCancelled { .. } => {
                            serde_json::to_string(&PlayerAgentFrame::SessionCancelled {
                                session_id,
            cancel_reason: None,
            cancelled_by: None,
                            })
                            .unwrap_or_default()
                        }
                        other => serde_json::to_string(&other).unwrap_or_default(),
                    };
                    if socket.send(Message::Text(json)).await.is_err() {
                        return SleepOutcome::Disconnected;
                    }
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => return SleepOutcome::Disconnected,
                    Some(Ok(Message::Ping(data))) => {
                        let _ = socket.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Text(text))) => {
                        // The agent may announce a completion flag push while
                        // we sleep between probes. Anything else (e.g. a late
                        // TestResult for an expired probe) stays ignored, as
                        // before.
                        if let Ok(PlayerAgentClientFrame::CompletionFlagPushed { path }) =
                            serde_json::from_str::<PlayerAgentClientFrame>(&text)
                        {
                            tracing::info!(
                                session_id = %session_id,
                                path = %path,
                                "player_agent: completion flag pushed; cutting inter-probe sleep"
                            );
                            return SleepOutcome::CompletionFlag;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    SleepOutcome::Elapsed
}

#[cfg(test)]
pub(crate) mod tests_memory {
    use super::*;

    pub(crate) fn test_model(command_template: &str, fixture_definitions: &str) -> tests::Model {
        tests::Model {
            id: Uuid::new_v4(),
            command_template: command_template.to_string(),
            answer_template: "Number(result.trim()) === 0".to_string(),
            fixture_definitions: fixture_definitions.to_string(),
            created_at: chrono::Utc::now(),
            session_id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            ordinal: 0,
            prompt: String::new(),
            description: None,
            probe_config: None,
            initiator: "system".to_string(),
            registered_by_judge_id: None,
        }
    }

    fn memory(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// The converted extreme-startup-cli shape: JS fixtures branch, memory
    /// value with a space arrives shell-quoted as one word (assign-then-
    /// invoke pattern in the task keeps it executable).
    #[tokio::test]
    async fn js_branch_renders_memory_command() {
        let t = test_model(
            "R={memory.command}\n$R -q \"{id}: what is 0 plus 0\"",
            r#"{"kind":"js","script":"({ id: \"abc\" })"}"#,
        );
        let mem = memory(&[("command", "sh answer.sh")]);
        let resolved = resolve_test_fixtures(&t, Uuid::new_v4(), &mem)
            .await
            .unwrap();
        assert_eq!(
            resolved.rendered_command,
            "R='sh answer.sh'\n$R -q \"abc: what is 0 plus 0\""
        );
    }

    /// The fizzbuzz regression: a comma-carrying `{list}` inside the
    /// template's own double quotes must arrive verbatim. Pre-quoting every
    /// scalar delivered `$R "'97,390,…'"` — the player's program saw literal
    /// quotes, and a correct answer failed every array probe.
    #[tokio::test]
    async fn js_branch_does_not_requote_inside_template_quotes() {
        let t = test_model(
            "R={memory.run}\n$R \"{list}\"",
            r#"{"kind":"js","script":"({ list: \"97,390,244\" })"}"#,
        );
        let mem = memory(&[("run", "sh answer.sh")]);
        let resolved = resolve_test_fixtures(&t, Uuid::new_v4(), &mem)
            .await
            .unwrap();
        assert_eq!(
            resolved.rendered_command,
            "R='sh answer.sh'\n$R \"97,390,244\""
        );
        // The persisted/shipped fixture values are the raw scalars, not
        // shell-quoted ones — the UI shows what the probe actually asked.
        assert_eq!(
            resolved.fixture_scalars.get("list").map(String::as_str),
            Some("97,390,244")
        );
    }

    /// Unknown memory keys stay literal in the JS branch (never silently
    /// substituted) — same behavior as unknown fixture tokens.
    #[tokio::test]
    async fn js_branch_leaves_unknown_memory_token_literal() {
        let t = test_model(
            "R={memory.nope}",
            r#"{"kind":"js","script":"({ id: \"abc\" })"}"#,
        );
        let mem = memory(&[("command", "sh answer.sh")]);
        let resolved = resolve_test_fixtures(&t, Uuid::new_v4(), &mem)
            .await
            .unwrap();
        assert_eq!(resolved.rendered_command, "R={memory.nope}");
    }

    /// FixtureDef-JSON branch (legacy; only `[]` occurs in practice —
    /// `FixtureDef`'s `deny_unknown_fields` + `flatten` cannot round-trip a
    /// non-empty list): `{memory.key}` is normalized and rendered.
    #[tokio::test]
    async fn fixture_def_branch_renders_memory() {
        let t = test_model("curl -s :{memory.port}/x", "[]");
        let mem = memory(&[("port", "8088")]);
        let resolved = resolve_test_fixtures(&t, Uuid::new_v4(), &mem)
            .await
            .unwrap();
        assert_eq!(resolved.rendered_command, "curl -s :8088/x");
    }

    /// Empty memory map: memory templates fail closed in the FixtureDef
    /// branch (Strict undefined) rather than dispatching a broken command.
    #[tokio::test]
    async fn fixture_def_branch_without_memory_fails_closed() {
        let t = test_model("curl -s :{{memory.port}}/x", "[]");
        let resolved = resolve_test_fixtures(&t, Uuid::new_v4(), &Default::default()).await;
        assert!(resolved.is_none());
    }
}

#[cfg(test)]
mod grade_stdout_tests {
    //! The three grading modes in one place. A solution must never pass one
    //! grader and fail another — the live loop, the execution judge and the
    //! server-side probes all come through here.
    use super::*;

    fn no_fixtures() -> (Vec<FixtureDef>, HashMap<String, FixtureSample>) {
        (Vec::new(), HashMap::new())
    }

    fn empty_memory() -> std::collections::BTreeMap<String, String> {
        std::collections::BTreeMap::new()
    }

    /// Mode 3: no template at all — the answer must equal the expected
    /// string exactly (trimmed by the caller).
    #[test]
    fn direct_equality_mode() {
        let (defs, values) = no_fixtures();
        let expected = Some("42".to_string());
        let grade = |stdout: &str| {
            grade_stdout(
                "",
                false,
                &None,
                &defs,
                &values,
                &expected,
                &empty_memory(),
                stdout,
                Some(0),
            )
            .0
        };
        assert!(grade("42"));
        assert!(!grade("43"));
        assert!(!grade(""), "an empty answer is not a match");

        // No expected value either: nothing can match.
        let (defs, values) = no_fixtures();
        assert!(
            !grade_stdout(
                "",
                false,
                &None,
                &defs,
                &values,
                &None,
                &empty_memory(),
                "42",
                Some(0)
            )
            .0
        );
    }

    /// Mode 2: a minijinja predicate expression over `result`.
    #[test]
    fn minijinja_template_mode() {
        let (defs, values) = no_fixtures();
        let grade = |stdout: &str| {
            grade_stdout(
                "result == '42'",
                false,
                &None,
                &defs,
                &values,
                &None,
                &empty_memory(),
                stdout,
                Some(0),
            )
            .0
        };
        assert!(grade("42"));
        assert!(!grade("41"));
    }

    /// A template that cannot be evaluated fails closed — never a free pass.
    #[test]
    fn unevaluatable_template_fails_closed() {
        let (defs, values) = no_fixtures();
        assert!(
            !grade_stdout(
                "result ==== not valid minijinja",
                false,
                &None,
                &defs,
                &values,
                &None,
                &empty_memory(),
                "anything",
                Some(0)
            )
            .0
        );
    }

    /// Mode 1: JS validation, with the assertEqual display values surfaced
    /// so the player's panel can show expected/actual.
    #[test]
    fn js_validation_mode_reports_display_values() {
        let (defs, values) = no_fixtures();
        let fx: Map<String, serde_json::Value> =
            serde_json::from_value(serde_json::json!({"a": 2, "b": 3})).expect("fixtures");
        let (pass, expected, actual) = grade_stdout(
            "assertEqual(Number(result.trim()), a + b)",
            true,
            &Some(fx.clone()),
            &defs,
            &values,
            &None,
            &empty_memory(),
            "5",
            Some(0),
        );
        assert!(pass, "2 + 3 = 5 passes");
        assert_eq!(expected.as_deref(), Some("5"));
        assert_eq!(actual.as_deref(), Some("5"));

        let (pass, expected, actual) = grade_stdout(
            "assertEqual(Number(result.trim()), a + b)",
            true,
            &Some(fx),
            &defs,
            &values,
            &None,
            &empty_memory(),
            "6",
            Some(0),
        );
        assert!(!pass);
        assert_eq!(expected.as_deref(), Some("5"));
        assert_eq!(
            actual.as_deref(),
            Some("6"),
            "the panel shows what came back"
        );
    }

    /// JS mode without resolved fixtures cannot grade — it must fail, not
    /// silently pass on an empty environment.
    #[test]
    fn js_validation_without_fixtures_fails() {
        let (defs, values) = no_fixtures();
        let (pass, _, _) = grade_stdout(
            "result.trim() === 'ok'",
            true,
            &None,
            &defs,
            &values,
            &None,
            &empty_memory(),
            "ok",
            Some(0),
        );
        assert!(!pass);
    }

    /// Probe points come from the section's `probe_config`; a legacy section
    /// (no config) scores by the task's own points, signalled by `None`.
    #[test]
    fn probe_points_reads_the_section_config() {
        let mut t = super::tests_memory::test_model("echo hi", "{}");
        assert_eq!(probe_points(&t), None, "no config → task-level scoring");

        t.probe_config =
            Some(serde_json::json!({"mode": "deterministic", "points": {"pass": 7, "fail": -3}}));
        let pts = probe_points(&t).expect("config present");
        assert_eq!(pts.pass, 7);
        assert_eq!(pts.fail, -3);

        // A config without a points block scores as a measurement (0/0).
        t.probe_config = Some(serde_json::json!({"mode": "deterministic"}));
        let pts = probe_points(&t).expect("config present");
        assert_eq!((pts.pass, pts.fail), (0, 0));

        // Unparseable config must not wedge the loop — measurement again.
        t.probe_config = Some(serde_json::json!({"mode": "deterministic", "points": "nonsense"}));
        let pts = probe_points(&t).expect("never None once a config exists");
        assert_eq!((pts.pass, pts.fail), (0, 0));

        // A capture-only probe (no explicit points) is an ordinary task
        // check — it must NOT fall to the 0/0 measurement default, or the
        // setup probe carrying the capture would silently stop paying.
        t.probe_config =
            Some(serde_json::json!({"mode": "deterministic", "capture_memory": "run"}));
        assert_eq!(
            probe_points(&t),
            None,
            "capture-only config → task-level scoring, not 0/0"
        );

        // …but a capture probe that DOES opt into points honors them.
        t.probe_config = Some(serde_json::json!({
            "mode": "deterministic", "capture_memory": "run", "points": {"pass": 5, "fail": 0}
        }));
        let pts = probe_points(&t).expect("config present");
        assert_eq!((pts.pass, pts.fail), (5, 0));
    }
}

#[cfg(test)]
mod unrunnable_reason_tests {
    //! A probe the agent never ran must fail on that fact alone. The bug this
    //! guards: a declined probe reached the validation script, and the decline
    //! notice satisfied predicates like `result.trim().length > 0`, so
    //! refusing to run the command paid full points.
    use super::*;

    #[test]
    fn a_probe_that_ran_is_graded_normally() {
        assert_eq!(unrunnable_reason(None, Some(0), "42"), None);
        assert_eq!(
            unrunnable_reason(None, Some(1), ""),
            None,
            "a command that ran and failed still has a real exit code"
        );
        assert_eq!(
            unrunnable_reason(None, Some(-1), "partial output"),
            None,
            "killed by a signal is a real run: the agent reports Some(-1)"
        );
        assert_eq!(
            unrunnable_reason(Some("   "), Some(0), "42"),
            None,
            "a blank error field is no error"
        );
    }

    #[test]
    fn a_declined_probe_is_unrunnable() {
        assert_eq!(
            unrunnable_reason(Some("declined by player"), Some(-1), PROBE_DECLINED_STDOUT)
                .as_deref(),
            Some("declined by player")
        );
        assert_eq!(
            unrunnable_reason(None, Some(-1), PROBE_DECLINED_STDOUT).as_deref(),
            Some("declined by player"),
            "an agent older than the error field is recognised by its placeholder"
        );
    }

    #[test]
    fn a_command_that_never_started_is_unrunnable() {
        // The protocol defines exit_code: None as "could not be started".
        assert!(unrunnable_reason(None, None, "").is_some());
        assert_eq!(
            unrunnable_reason(Some("No such file or directory"), None, "").as_deref(),
            Some("No such file or directory"),
            "the agent's own reason wins over the generic one"
        );
    }

    /// The regression itself: the validation script would have passed the
    /// decline notice, so the guard has to run before it.
    #[test]
    fn the_decline_notice_would_otherwise_pass_a_permissive_validation() {
        let fx: Map<String, serde_json::Value> =
            serde_json::from_value(serde_json::json!({})).expect("fixtures");
        let (pass, _, _) = grade_stdout(
            "result.trim().length > 0",
            true,
            &Some(fx),
            &[],
            &HashMap::new(),
            &None,
            &std::collections::BTreeMap::new(),
            PROBE_DECLINED_STDOUT,
            Some(-1),
        );
        assert!(
            pass,
            "the validation alone cannot tell a decline from an answer"
        );
        assert!(
            unrunnable_reason(None, Some(-1), PROBE_DECLINED_STDOUT).is_some(),
            "so the guard must catch it first"
        );
    }
}
