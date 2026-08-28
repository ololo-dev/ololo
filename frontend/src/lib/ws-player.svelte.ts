import type {
  PlayerFrame,
  PlayerSnapshotPayload,
  PlayerProbeEntry,
  ProbeUpdatedPayload,
  PlayerTaskSummaryEntry,
  PlayerJudgeScoredPayload,
  PlayerJudgeStatusPayload,
} from "$lib/types/arena.js";
import {
  UNBOUNDED_RECONNECT,
  createWsConnection,
  type WsConnection,
} from "$lib/ws/connection.svelte.js";

const GAP_THRESHOLD = 10;

export class WsPlayerClient {
  private readonly playerId: string;
  private readonly sessionCode: string;
  private readonly token: string;
  private readonly serverBase: string;
  private readonly _conn: WsConnection;

  get connected() {
    return this._conn.connected;
  }
  snapshot = $state<PlayerSnapshotPayload | null>(null);
  error = $state<string | null>(null);
  sessionFinished = $state(false);
  sessionPaused = $state(false);
  /** Live agent presence; null until the snapshot or a presence frame tells us. */
  agentConnected = $state<boolean | null>(null);
  /** When the session ended by cancellation, why and by whom. */
  cancelReason = $state<string | null>(null);
  cancelledBy = $state<string | null>(null);
  judgeResults = $state<PlayerJudgeScoredPayload[]>([]);
  judgeStatuses = $state<PlayerJudgeStatusPayload[]>([]);

  private lastSeq: number = 0;
  private _refreshTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(playerId: string, sessionCode: string, token: string, serverBase = "") {
    this.playerId = playerId;
    this.sessionCode = sessionCode;
    this.token = token;
    this.serverBase = serverBase;

    const path = `/ws/player/${playerId}?session_code=${encodeURIComponent(sessionCode)}&token=${encodeURIComponent(token)}`;
    this._conn = createWsConnection({
      path,
      onMessage: (raw) => this._onRaw(raw),
      // Keep retrying for as long as the page holds the client: this socket
      // is the live feed for probes and judge verdicts, and a single dropped
      // reconnect used to freeze the page until a manual reload. The page
      // disconnects the client when the session ends.
      reconnect: UNBOUNDED_RECONNECT,
    });
  }

  connect(): void {
    this._conn.connect();
  }

  disconnect(): void {
    if (this._refreshTimer !== null) {
      clearTimeout(this._refreshTimer);
      this._refreshTimer = null;
    }
    this._conn.disconnect();
  }

  /** Re-fetch the full snapshot now (evaluations, artifacts, judge details). */
  refreshSnapshot(): void {
    this._requestFreshSnapshot();
  }

  /** Coalesce snapshot re-fetches: judge panels land several verdicts within
   *  moments of each other, and one fetch covers them all. */
  private _scheduleRefresh(delayMs: number): void {
    if (this._refreshTimer !== null) return;
    this._refreshTimer = setTimeout(() => {
      this._refreshTimer = null;
      this._requestFreshSnapshot();
    }, delayMs);
  }

  private _onRaw(raw: string): void {
    try {
      const frame = JSON.parse(raw) as PlayerFrame;
      this._handleFrame(frame);
    } catch {
      // ignore malformed frames
    }
  }

  private _handleFrame(frame: PlayerFrame): void {
    const seq = "seq" in frame ? frame.seq : 0;

    switch (frame.type) {
      case "player_snapshot": {
        const { type: _, ...payload } = frame;
        this.snapshot = payload as PlayerSnapshotPayload;
        this.lastSeq = payload.last_seq;
        this.sessionPaused = this.snapshot.session_status === "paused";
        this.sessionFinished =
          this.snapshot.session_status === "finished" ||
          this.snapshot.session_status === "cancelled";
        this.agentConnected = this.snapshot.agent_connected ?? null;
        this._seedJudgeResults(this.snapshot.judge_results ?? []);
        this._seedJudgeStatuses(this.snapshot.judge_statuses ?? []);
        break;
      }
      case "probe_updated": {
        if (seq <= this.lastSeq) break;
        if (!this.snapshot) break;
        this._applySeq(seq);
        this._upsertProbe(frame);
        this.snapshot = {
          ...this.snapshot,
          score: frame.score,
          rank: frame.rank,
          next_probe_at: frame.next_probe_at ?? this.snapshot.next_probe_at,
        };
        break;
      }
      case "player_task_delta": {
        if (seq <= this.lastSeq) break;
        if (!this.snapshot) break;
        this._applySeq(seq);
        this.snapshot = {
          ...this.snapshot,
          tasks: this.snapshot.tasks.map((task) =>
            task.task_id === frame.task_id
              ? {
                  ...task,
                  result: frame.result ?? task.result,
                  // A delta that carries a result is the scheduler letting go
                  // of the task — its null scheduler_state means "cleared",
                  // not "unchanged" (a stale "judging" would linger forever).
                  scheduler_state: frame.result
                    ? (frame.scheduler_state ?? null)
                    : (frame.scheduler_state ?? task.scheduler_state),
                }
              : task,
          ),
          score: frame.score,
          rank: frame.rank,
        };
        break;
      }
      case "session_complete": {
        if (seq <= this.lastSeq) break;
        this._applySeq(seq);
        this.sessionFinished = true;
        this.sessionPaused = false;
        if (this.snapshot) {
          this.snapshot = {
            ...this.snapshot,
            score: frame.score,
            rank: frame.rank,
          };
        }
        break;
      }
      case "agent_presence": {
        // No seq: presence frames ride outside the scored-event stream, like
        // the judge lifecycle frames.
        this.agentConnected = frame.connected;
        break;
      }
      case "session_status_change": {
        const status = (frame as { status: string }).status;
        this.sessionPaused = status === "paused";
        this.sessionFinished = status === "finished" || status === "cancelled";
        if (status === "cancelled") {
          this.cancelReason = frame.cancel_reason ?? null;
          this.cancelledBy = frame.cancelled_by ?? null;
        }
        if (this.snapshot) {
          this.snapshot = {
            ...this.snapshot,
            session_status: status as PlayerSnapshotPayload["session_status"],
          };
        }
        break;
      }
      case "player_error": {
        if (seq <= this.lastSeq) break;
        this._applySeq(seq);
        this.error = frame.message;
        break;
      }
      case "task_revealed": {
        if (seq <= this.lastSeq) break;
        if (!this.snapshot) break;
        this._applySeq(seq);
        // The scheduler advanced: the previous task just finalized, and its
        // completion bonus / final totals only exist server-side. Reconcile
        // with a fresh snapshot (async, keeps the optimistic update below).
        this._requestFreshSnapshot();
        // Server reveals next task when scheduler advances (task done).
        // Finalize the previously-active task: pass or fail based on probes.
        const currentProbes = this.snapshot.probes;
        let tasks: PlayerTaskSummaryEntry[];
        const taskIdx = this.snapshot.tasks.findIndex((t) => t.task_id === frame.task.task_id);
        if (taskIdx >= 0) {
          tasks = this.snapshot.tasks.map((t, i) =>
            i === taskIdx ? frame.task : finalizeActiveTask(t, currentProbes),
          );
        } else {
          tasks = [
            ...this.snapshot.tasks.map((t) => finalizeActiveTask(t, currentProbes)),
            frame.task,
          ];
        }
        this.snapshot = {
          ...this.snapshot,
          tasks,
          total_tasks: frame.total_tasks,
        };
        break;
      }
      case "score_rank_updated": {
        if (seq <= this.lastSeq) break;
        if (!this.snapshot) break;
        this._applySeq(seq);
        this.snapshot = {
          ...this.snapshot,
          score: frame.score,
          rank: frame.rank,
        };
        break;
      }
      case "judge_scored": {
        const { type: _, ...payload } = frame;
        const verdict = payload as PlayerJudgeScoredPayload;
        // Patch the task's running total: a rerun for the same (task, judge)
        // replaces the prior verdict, so apply the difference, not the sum.
        const prior = this.judgeResults.find(
          (r) => r.task_id === verdict.task_id && r.judge_slug === verdict.judge_slug,
        );
        const diff = verdict.point_delta - (prior?.point_delta ?? 0);
        if (diff !== 0 && this.snapshot) {
          this.snapshot = {
            ...this.snapshot,
            tasks: this.snapshot.tasks.map((t) =>
              t.task_id === verdict.task_id
                ? { ...t, total_points: (t.total_points ?? 0) + diff }
                : t,
            ),
          };
        }
        this._seedJudgeResults([verdict]);
        this._seedJudgeStatuses([
          {
            task_id: verdict.task_id,
            judge_slug: verdict.judge_slug,
            judge_name: verdict.judge_name,
            status: "scored",
            error: null,
            updated_at: verdict.created_at,
          },
        ]);
        // The verdict frame carries the headline only — the criteria sheet,
        // judge_result_id, and any captures live in the snapshot.
        this._scheduleRefresh(400);
        break;
      }
      case "judge_started":
      case "judge_failed": {
        const { type: _, ...payload } = frame;
        this._seedJudgeStatuses([payload as PlayerJudgeStatusPayload]);
        if (frame.type === "judge_failed") this._scheduleRefresh(400);
        break;
      }
      case "artifact_awaited":
      case "evaluation_ready":
      case "session_report_ready": {
        // Evaluation state (pending artifact requests, criteria scores,
        // screenshots, repo images) and the session report exist only in the
        // snapshot — re-fetch it. The report matters most here: it is written
        // after the session finished, when the reconcile poll has stopped, so
        // without this it waited for a manual reload.
        this._scheduleRefresh(0);
        break;
      }
    }
  }

  /** Merge judge results, deduping on (task_id, judge_slug): the snapshot
   *  carries persisted results and live judge_scored frames re-deliver the
   *  same verdict (judge reruns upsert, so keep the latest per key). */
  private _seedJudgeResults(incoming: PlayerJudgeScoredPayload[]): void {
    if (incoming.length === 0) return;
    const byKey = new Map<string, PlayerJudgeScoredPayload>();
    for (const r of [...this.judgeResults, ...incoming]) {
      byKey.set(`${r.task_id}:${r.judge_slug}`, r);
    }
    this.judgeResults = [...byKey.values()];
  }

  /** Merge judge lifecycle entries, deduping on (task_id, judge_slug):
   *  the snapshot lists every attachment and live judge_started /
   *  judge_failed / judge_scored frames advance an entry's status. */
  private _seedJudgeStatuses(incoming: PlayerJudgeStatusPayload[]): void {
    if (incoming.length === 0) return;
    const byKey = new Map<string, PlayerJudgeStatusPayload>();
    for (const s of [...this.judgeStatuses, ...incoming]) {
      byKey.set(`${s.task_id}:${s.judge_slug}`, s);
    }
    this.judgeStatuses = [...byKey.values()];
  }

  private _applySeq(seq: number): void {
    const gap = seq - this.lastSeq;
    if (gap > GAP_THRESHOLD) {
      this._requestFreshSnapshot();
    }
    this.lastSeq = seq;
  }

  private _requestFreshSnapshot(): void {
    const base = this.serverBase || "";
    const url = `${base}/api/sessions/${encodeURIComponent(this.sessionCode)}/players/${encodeURIComponent(this.playerId)}`;
    fetch(url.replace(/^ws/, "http"))
      .then((res) => res.json())
      .then((data) => {
        if (data && data.player_id) {
          this.snapshot = data as PlayerSnapshotPayload;
          this.lastSeq = data.last_seq ?? this.lastSeq;
          this._seedJudgeResults(this.snapshot.judge_results ?? []);
          this._seedJudgeStatuses(this.snapshot.judge_statuses ?? []);
        }
      })
      .catch(() => {
        // fetch failed — keep existing state
      });
  }

  private _upsertProbe(payload: ProbeUpdatedPayload): void {
    if (!this.snapshot) return;

    const probeEntry: PlayerProbeEntry = {
      id: payload.probe_id,
      task_id: payload.task_id,
      task_title: payload.task_title,
      task_ordinal: payload.task_ordinal,
      adapted_test_id: payload.adapted_test_id,
      test_ordinal: payload.test_ordinal,
      label: payload.label,
      description: payload.description,
      test_command: payload.test_command,
      attempt: payload.attempt,
      rendered_command: payload.rendered_command,
      fixture_values: payload.fixture_values,
      expected_answer: payload.expected_answer,
      state: payload.state,
      outcome: payload.outcome,
      actual: payload.actual,
      expected: payload.expected,
      exit_code: payload.exit_code,
      duration_ms: payload.duration_ms,
      dispatched_at: payload.dispatched_at,
      deadline_at: payload.deadline_at,
      resolved_at: payload.resolved_at,
      result: payload.outcome
        ? { status: payload.outcome, expected: payload.expected, actual: payload.actual }
        : null,
      point_delta: payload.point_delta,
      updated_at: payload.updated_at,
    };

    const idx = this.snapshot.probes.findIndex((p) => p.id === payload.probe_id);
    let probes: PlayerProbeEntry[];
    let pointDiff = payload.point_delta;
    if (idx >= 0) {
      pointDiff = payload.point_delta - this.snapshot.probes[idx].point_delta;
      probes = this.snapshot.probes.map((p, i) => (i === idx ? probeEntry : p));
    } else {
      probes = [...this.snapshot.probes, probeEntry];
    }

    let tasks = this.snapshot.tasks;
    // Keep the task's running total in sync with the probe's graded delta
    // (re-delivered frames carry the same delta, so apply the difference).
    if (pointDiff !== 0) {
      tasks = tasks.map((t) =>
        t.task_id === payload.task_id
          ? { ...t, total_points: (t.total_points ?? 0) + pointDiff }
          : t,
      );
    }
    const taskIdx = tasks.findIndex((t) => t.task_id === payload.task_id);
    if (taskIdx < 0) {
      const newTask: PlayerTaskSummaryEntry = {
        task_id: payload.task_id,
        ordinal: payload.task_ordinal,
        title: payload.task_title,
        content: "",
        tags: [],
        adapted_content: "",
        result: null,
        scheduler_state: null,
        total_points: payload.point_delta,
      };
      tasks = [...tasks, newTask];
    }

    this.snapshot = { ...this.snapshot, probes, tasks };
  }
}

function finalizeActiveTask(
  t: PlayerTaskSummaryEntry,
  probes: PlayerProbeEntry[],
): PlayerTaskSummaryEntry {
  if (t.scheduler_state == null) return { ...t, scheduler_state: null };
  if (t.result != null) return { ...t, scheduler_state: null };

  // Determine pass/fail from the task's probes.
  // The server advances the scheduler when the task is done (all test types
  // passed, or failed/expired). Match the SSR logic: if any probe for this
  // task failed, mark failed; otherwise completed.
  const taskProbes = probes.filter((p) => p.task_id === t.task_id);
  const hasFail = taskProbes.some((p) => p.outcome === "error" || p.outcome === "no_response");
  return {
    ...t,
    scheduler_state: null,
    result: {
      status: hasFail ? "failed" : "completed",
      submitted_answer: null,
      correct_answer: null,
      score_delta: 0,
      evaluated_at: null,
    },
  };
}
