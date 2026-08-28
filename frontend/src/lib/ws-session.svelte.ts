/**
 * Typed WebSocket client for the session dashboard endpoint.
 * Connects to `GET /ws/s/:join_code` with `?token=<JWT>` for browser
 * clients (browsers cannot set custom headers on WS upgrade).
 *
 * Usage:
 *   const session = new WsSessionClient(code, token, serverBase);
 *   session.connect();
 *   // read session.connected, session.phase, session.leaderboard, etc.
 *   session.disconnect();
 *
 * Transport (open/close/backoff) is delegated to `createWsConnection`.
 */

import type {
  ActivityEvent,
  ArenaFrame,
  LeaderboardEntry,
  MemberInfo,
  PlayerSummary,
  ScoreHistoryPoint,
} from "$lib/types/arena";
import { backendPhaseToUiPhase, type UiSessionPhase } from "$lib/session-phase";
import type { PlayerProgressState } from "$lib/session-view-model";
import { createWsConnection, type WsConnection } from "$lib/ws/connection.svelte";

export type AgentProgressState = PlayerProgressState;

export type SessionPhase = UiSessionPhase;

const KNOWN_FRAME_TYPES = new Set([
  "test_push",
  "session_complete",
  "session_settled",
  "session_cancelled",
  "leaderboard_update",
  "player_progress_update",
  "agent_progress_update",
  "test_result",
  "heartbeat",
  "lobby_countdown",
  "running_countdown",
  "session_started",
  "member_list",
  "member_joined",
  "session_snapshot",
  "player_disconnected",
  "member_disconnected",
  "project_session_update",
  "admin_adapted_tasks_snapshot",
  "admin_adapted_task_updated",
  "user_players_snapshot",
  "task_started",
  "task_scored",
  "artifact_received",
]);

export class WsSessionClient {
  private readonly joinCode: string;
  private readonly token: string | null;
  private readonly serverBase: string;
  private readonly _conn: WsConnection;

  // --- reactive state ---
  get connected() {
    return this._conn.connected;
  }
  phase = $state<SessionPhase>("lobby");
  /** True once the server said every owed judge run settled: standings final. */
  settled = $state(false);
  /** True when the session ended by cancellation (admin or idle sweep), not by finishing. */
  endedCancelled = $state(false);
  cancelReason = $state<string | null>(null);
  cancelledBy = $state<string | null>(null);
  isPaused = $derived(this.phase === "paused");
  leaderboard = $state<LeaderboardEntry[]>([]);
  countdownSecs = $state<number | null>(null);
  runningCountdownSecs = $state<number | null>(null);
  participants = $state<MemberInfo[]>([]);
  scoreHistory = $state<ScoreHistoryPoint[]>([]);
  startedAt = $state<Date | null>(null);
  /** Per-player task progress, keyed by player_id (UUID string). */
  agentProgress = $state<Record<string, AgentProgressState>>({});
  userPlayers = $state<PlayerSummary[]>([]);
  activityLog = $state<ActivityEvent[]>([]);
  protocolMismatch = $state(false);
  degraded = $state<string | null>(null);

  private _startedAtMs: number | null = null;
  private _lastVersion = 0;

  constructor(joinCode: string, token: string | null, serverBase = "") {
    this.joinCode = joinCode;
    this.token = token;
    this.serverBase = serverBase;

    const path = token
      ? `/ws/s/${joinCode}?token=${encodeURIComponent(token)}`
      : `/ws/s/${joinCode}/observe`;

    this._conn = createWsConnection({
      path,
      onMessage: (raw) => this._onRaw(raw),
      reconnect: { initialMs: 1_000, maxMs: 30_000, multiplier: 2, maxAttempts: 8 },
    });
  }

  connect(): void {
    this._conn.connect();
  }

  disconnect(): void {
    this._conn.disconnect();
  }

  private _onRaw(raw: string): void {
    let frameType = "";
    try {
      frameType = (JSON.parse(raw) as { type?: string }).type ?? "";
    } catch {
      this.protocolMismatch = true;
      return;
    }
    if (!KNOWN_FRAME_TYPES.has(frameType)) {
      this.protocolMismatch = true;
      return;
    }
    let frame: ArenaFrame;
    try {
      frame = JSON.parse(raw) as ArenaFrame;
    } catch {
      this.protocolMismatch = true;
      return;
    }
    this._handleFrame(frame);
  }

  private _handleFrame(frame: ArenaFrame): void {
    // Event frames (task_started, task_scored) carry a version for provenance
    // but must NOT participate in stale-frame dedup — they are append-only
    // events, not state snapshots that can be superseded.
    const isEventFrame =
      frame.type === "task_started" ||
      frame.type === "task_scored" ||
      frame.type === "artifact_received";
    const frameVersion =
      typeof (frame as { version?: unknown }).version === "number"
        ? (frame as { version: number }).version
        : null;
    if (!isEventFrame && frameVersion !== null && frameVersion <= this._lastVersion) {
      return;
    }
    if (!isEventFrame && frameVersion !== null) {
      this._lastVersion = frameVersion;
    }

    switch (frame.type) {
      case "lobby_countdown":
        this.countdownSecs = frame.seconds_remaining;
        break;

      case "running_countdown":
        this.runningCountdownSecs = frame.seconds_remaining;
        break;

      case "session_started":
        this.phase = backendPhaseToUiPhase("running");
        this._startedAtMs = Date.now();
        this.startedAt = new Date();
        this.countdownSecs = null;
        break;

      case "session_complete":
        this.phase = backendPhaseToUiPhase("finished");
        this.runningCountdownSecs = null;
        if ((frame as { reason?: string }).reason?.startsWith("cancelled")) {
          this.endedCancelled = true;
        }
        break;

      case "session_settled":
        this.settled = true;
        break;

      case "session_cancelled":
        this.phase = backendPhaseToUiPhase("finished");
        this.runningCountdownSecs = null;
        this.endedCancelled = true;
        this.cancelReason = (frame as { cancel_reason?: string }).cancel_reason ?? null;
        this.cancelledBy = (frame as { cancelled_by?: string }).cancelled_by ?? null;
        break;

      case "leaderboard_update": {
        this.leaderboard = frame.entries.map((entry) => ({
          ...entry,
          player_id: entry.player_id ?? entry.agent_id ?? "",
        }));
        const elapsed = this._startedAtMs !== null ? (Date.now() - this._startedAtMs) / 1000 : 0;
        const scores: Record<string, number> = {};
        for (const e of this.leaderboard) {
          scores[e.player_id] = e.total_points;
        }
        this.scoreHistory = [...this.scoreHistory, { t: elapsed, scores }];
        break;
      }

      case "player_progress_update": {
        const { player_id, current_task_id } = frame;
        const prev = this.agentProgress[player_id];
        const visited = prev ? [...prev.visited_task_ids] : [];
        if (prev?.current_task_id && prev.current_task_id !== current_task_id) {
          if (!visited.includes(prev.current_task_id)) {
            visited.push(prev.current_task_id);
          }
        }
        this.agentProgress = {
          ...this.agentProgress,
          [player_id]: { current_task_id, visited_task_ids: visited },
        };
        break;
      }

      case "agent_progress_update": {
        const { agent_id, current_task_id } = frame;
        const prev = this.agentProgress[agent_id];
        const visited = prev ? [...prev.visited_task_ids] : [];
        // Accumulate: if moving off a task, add it to visited history.
        if (prev?.current_task_id && prev.current_task_id !== current_task_id) {
          if (!visited.includes(prev.current_task_id)) {
            visited.push(prev.current_task_id);
          }
        }
        this.agentProgress = {
          ...this.agentProgress,
          [agent_id]: { current_task_id, visited_task_ids: visited },
        };
        break;
      }

      case "session_snapshot": {
        if (typeof frame.version === "number") {
          if (frame.version < this._lastVersion) {
            break;
          }
          this._lastVersion = frame.version;
        }
        this.phase = backendPhaseToUiPhase(frame.phase);
        this.participants = frame.participants;
        this.leaderboard = frame.leaderboard;
        if (frame.started_at !== null) {
          this.startedAt = new Date(frame.started_at);
          this._startedAtMs = this.startedAt.getTime();
        }
        // Seed activityLog from the snapshot so a page refresh shows the
        // full history immediately rather than waiting for live events.
        if (frame.activity && frame.activity.length > 0) {
          this.activityLog = frame.activity.map((e) => ({
            kind: e.event_kind as ActivityEvent["kind"],
            player_id: e.player_id,
            player_display_name: e.player_display_name,
            task_id: e.task_id,
            task_ordinal: e.task_ordinal,
            task_title: e.task_title,
            judge_name: e.judge_name,
            point_delta: e.point_delta,
            // The verdict payload (criteria sheet, feedback). Dropping this
            // made judge descriptions vanish on refresh while live frames
            // kept them.
            detail: e.detail ?? null,
            timestamp: e.timestamp,
            version: e.version,
          }));
        }
        // Seed scoreHistory from snapshot on first connect so the chart shows
        // existing scores immediately rather than waiting for the next probe.
        if (this.scoreHistory.length === 0) {
          if (frame.score_history && frame.score_history.length > 0) {
            this.scoreHistory = frame.score_history;
          } else if (frame.phase === "running" && this.leaderboard.length > 0) {
            const elapsed =
              this._startedAtMs !== null ? (Date.now() - this._startedAtMs) / 1000 : 0;
            const scores: Record<string, number> = {};
            for (const e of this.leaderboard) {
              scores[e.player_id] = e.total_points;
            }
            this.scoreHistory = [{ t: elapsed, scores }];
          }
        }
        break;
      }

      case "player_disconnected":
        this.participants = this.participants.filter(
          (p) => (p.player_id ?? p.user_id) !== frame.player_id,
        );
        break;

      case "member_disconnected":
        this.participants = this.participants.filter((p) => p.user_id !== frame.agent_id);
        break;

      case "member_joined": {
        const { user_id, display_name, joined_at } = frame;
        const playerId = frame.player_id ?? null;
        const existing = this.participants.find((p) => p.user_id === user_id);
        if (existing) {
          // Metadata refresh (e.g. ololo PATCHed its AI agent): update
          // the existing entry in place so lobby observers see the agent
          // label without a reconnect.
          this.participants = this.participants.map((p) =>
            p.user_id === user_id
              ? {
                  ...p,
                  player_id: playerId ?? p.player_id ?? null,
                  display_name,
                  joined_at,
                  avatar_url: frame.avatar_url ?? p.avatar_url ?? null,
                  fingerprint: frame.fingerprint ?? p.fingerprint ?? null,
                  username: frame.username ?? p.username ?? null,
                  agent_display_name: frame.agent_display_name ?? p.agent_display_name ?? null,
                }
              : p,
          );
        } else {
          this.participants = [
            ...this.participants,
            {
              user_id,
              player_id: playerId,
              display_name,
              joined_at,
              avatar_url: frame.avatar_url ?? null,
              fingerprint: frame.fingerprint ?? null,
              username: frame.username ?? null,
              agent_display_name: frame.agent_display_name ?? null,
            },
          ];
        }
        break;
      }

      case "heartbeat":
        break;

      case "user_players_snapshot":
        this.userPlayers = frame.players;
        break;

      case "project_session_update":
        if (frame.status) {
          this.phase = backendPhaseToUiPhase(frame.status);
          if (frame.status === "cancelled") {
            this.endedCancelled = true;
            this.cancelReason = (frame as { cancel_reason?: string }).cancel_reason ?? null;
            this.cancelledBy = (frame as { cancelled_by?: string }).cancelled_by ?? null;
          }
        }
        break;

      case "task_started": {
        // A task starts once per player per session — drop repeats (e.g. an
        // event already seeded from the snapshot arriving again live).
        if (
          this.activityLog.some(
            (e) =>
              e.kind === "task_started" &&
              e.player_id === frame.player_id &&
              e.task_id === frame.task_id,
          )
        ) {
          break;
        }
        const entry: ActivityEvent = {
          kind: "task_started",
          player_id: frame.player_id,
          player_display_name: frame.player_display_name,
          task_id: frame.task_id,
          task_ordinal: frame.task_ordinal,
          task_title: frame.task_title,
          judge_name: null,
          point_delta: null,
          timestamp: frame.timestamp,
          version: frame.version,
        };
        this.activityLog = [...this.activityLog, entry];
        if (this.activityLog.length > 500) {
          this.activityLog = this.activityLog.slice(this.activityLog.length - 500);
        }
        break;
      }

      case "artifact_received": {
        // One artifact per probe — the resolver records it once, but a
        // snapshot-seeded entry may arrive again live.
        if (
          this.activityLog.some(
            (e) => e.kind === "artifact_received" && e.detail?.probe_id === frame.probe_id,
          )
        ) {
          break;
        }
        const entry: ActivityEvent = {
          kind: "artifact_received",
          player_id: frame.player_id,
          player_display_name: frame.player_display_name,
          task_id: frame.task_id,
          task_ordinal: frame.task_ordinal,
          task_title: frame.task_title,
          judge_name: null,
          point_delta: null,
          detail: {
            probe_id: frame.probe_id,
            path: frame.path,
            size: frame.size,
            content_type: frame.content_type,
            within_cap: frame.within_cap,
            files: frame.files,
          },
          timestamp: frame.timestamp,
          version: frame.version,
        };
        this.activityLog = [...this.activityLog, entry];
        if (this.activityLog.length > 500) {
          this.activityLog = this.activityLog.slice(this.activityLog.length - 500);
        }
        break;
      }

      case "task_scored": {
        const judgeName = frame.judge_name || null;
        const entry: ActivityEvent = {
          kind: "task_scored",
          player_id: frame.player_id,
          player_display_name: frame.player_display_name,
          task_id: frame.task_id,
          task_ordinal: frame.task_ordinal,
          task_title: frame.task_title,
          judge_name: judgeName,
          point_delta: frame.point_delta,
          detail: frame.detail ?? null,
          timestamp: frame.timestamp,
          version: frame.version,
        };
        // A probe-pass "implemented Task N" line (no judge) is re-emitted on
        // every scoring cycle but reflects no new award — collapse to the
        // first per (player, task), mirroring task_started and the server-side
        // dedupe_activity_feed.
        if (
          !judgeName &&
          this.activityLog.some(
            (e) =>
              e.kind === "task_scored" &&
              !e.judge_name &&
              e.player_id === frame.player_id &&
              e.task_id === frame.task_id,
          )
        ) {
          break;
        }
        // A judge verdict is one row per (player, task, judge): different
        // judges keep their own rows, but a re-run of the SAME judge (recovery
        // sweep, manual re-judge) replaces its prior verdict rather than
        // stacking a duplicate. Mirrors the server-side "keep latest" rule.
        if (judgeName) {
          const at = this.activityLog.findIndex(
            (e) =>
              e.kind === "task_scored" &&
              e.judge_name === judgeName &&
              e.player_id === frame.player_id &&
              e.task_id === frame.task_id,
          );
          if (at !== -1) {
            this.activityLog = [
              ...this.activityLog.slice(0, at),
              entry,
              ...this.activityLog.slice(at + 1),
            ];
            break;
          }
        }
        this.activityLog = [...this.activityLog, entry];
        if (this.activityLog.length > 500) {
          this.activityLog = this.activityLog.slice(this.activityLog.length - 500);
        }
        break;
      }

      default:
        this.protocolMismatch = true;
        break;
    }
  }
}
