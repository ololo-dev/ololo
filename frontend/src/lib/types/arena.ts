export type AgentRunStatus = "awaiting_result" | "backoff" | "completed" | "failed";

export type AdaptedTaskStatus = "pending" | "ready" | "failed";

export interface AdminTaskEntry {
  task_id: string;
  task_order: number;
  title: string;
  status: AdaptedTaskStatus;
  adaptation_attempts: number;
  adapted_content?: string;
}

export interface AdminAdaptedTaskView {
  player_id: string;
  player_display_name: string;
  tasks: AdminTaskEntry[];
}

export interface LeaderboardEntry {
  player_id: string;
  // Backward-compat alias for stale callers not yet migrated.
  agent_id?: string;
  display_name: string;
  /** AI coding agent the participant is running, if reported. */
  agent_display_name?: string | null;
  total_points: number;
  tests_passed: number;
  total_wall_ms: number;
}

/**
 * Derived completion state of a player within a session. The key is absent
 * (never null) on list endpoints and for participants without computed data —
 * only detail/snapshot paths carry it.
 */
export type PlayerCompletionStatus = "in_progress" | "awaiting_judges" | "completed";

export interface MemberInfo {
  user_id: string;
  /** Player row id in this session, when known — the id the leaderboard is keyed by. */
  player_id?: string | null;
  display_name: string;
  joined_at: string;
  avatar_url?: string | null;
  fingerprint?: string | null;
  username?: string | null;
  agent_display_name?: string | null;
  completion_status?: PlayerCompletionStatus;
}

export interface SessionActivityEvent {
  event_kind: string;
  player_id: string;
  player_display_name: string;
  task_id: string;
  task_ordinal: number;
  task_title: string;
  judge_name: string | null;
  point_delta: number | null;
  detail?: ActivityDetail | null;
  timestamp: string;
  version: number;
}

export interface SessionSnapshotPayload {
  session_id: string;
  phase: string;
  version?: number;
  participants: MemberInfo[];
  leaderboard: LeaderboardEntry[];
  started_at: string | null;
  activity?: SessionActivityEvent[];
  score_history?: ScoreHistoryPoint[];
}

export interface ScoreHistoryPoint {
  t: number;
  scores: Record<string, number>;
}

export interface SessionInfo {
  id: string;
  join_code: string;
  state: string;
}

export interface PlayerSummary {
  player_id: string;
  /** The user who owns this player; null for anonymous/unlinked players. */
  user_id: string | null;
  display_name: string;
  fingerprint: string | null;
  joined_at: string;
  reconnected_at: string | null;
  revoked_at: string | null;
}

export interface PlayerTaskResult {
  status: string; // "correct" | "incorrect" | "pending" | "skipped"
  submitted_answer: string | null;
  correct_answer: string | null;
  score_delta: number;
  evaluated_at: string | null;
}

export interface PlayerSchedulerState {
  state: string; // "waiting" | "active" | "completed" | "expired"
  activated_at: string | null;
  deadline_at: string | null;
}

export type ProbeState = "dispatched" | "resolved";

/** The probe fields shared by the snapshot entry and the live wire frame —
 *  one shape, so the store and `ProbeUpdatedPayload` cannot drift apart. */
export interface ProbeFields {
  task_id: string;
  task_title: string;
  task_ordinal: number;
  adapted_test_id: string;
  /** Ordinal of the probe type (tests row) within the task, when known. */
  test_ordinal?: number | null;
  /** Human label of the test — its `## ` heading from the task definition.
   *  Absent on legacy sessions adapted before labels were carried. */
  label?: string | null;
  /** The test section's prose — what the check verifies in the author's
   *  words (or the judge's instruction for judge-registered probes). */
  description?: string | null;
  test_command: string;
  attempt: number;
  rendered_command: string;
  fixture_values: string | null;
  expected_answer: string | null;
  state: ProbeState;
  outcome: string | null;
  actual: string | null;
  expected: string | null;
  exit_code: number | null;
  duration_ms: number | null;
  dispatched_at: string | null;
  deadline_at: string | null;
  resolved_at: string | null;
  point_delta: number;
}

export interface PlayerProbeEntry extends ProbeFields {
  id: string;
  result: { status: string; expected: string | null; actual: string | null } | null;
  updated_at: string | null;
}

export interface PlayerTaskSummaryEntry {
  task_id: string;
  ordinal: number;
  title: string;
  content: string;
  tags: string[];
  adapted_content: string;
  result: PlayerTaskResult | null;
  scheduler_state: PlayerSchedulerState | null;
  /** Sum of all point deltas on this task (probes + bonus + judges). */
  total_points?: number;
  /** Completion-bonus portion of total_points, labelled separately in the UI. */
  bonus_points?: number;
}

export interface PlayerSnapshotPayload {
  player_id: string;
  display_name: string;
  /** Avatar URL of the account that owns this player, if set. */
  avatar_url?: string | null;
  /** AI coding agent the participant is running, if reported. */
  agent_display_name?: string | null;
  score: number;
  rank: number;
  last_seq: number;
  probes: PlayerProbeEntry[];
  tasks: PlayerTaskSummaryEntry[];
  total_tasks: number;
  next_probe_at: string | null;
  session_started_at: string | null;
  session_ends_at: string | null;
  session_status: "lobby" | "running" | "paused" | "finished" | "cancelled";
  /** Whether the player's ololo agent socket is connected right now. */
  agent_connected?: boolean;
  completion_status?: PlayerCompletionStatus;
  /** Persisted judge verdicts; live updates arrive as judge_scored frames. */
  judge_results?: PlayerJudgeScoredPayload[];
  /** Lifecycle of every judge attached to a revealed task
   *  (pending/running/scored/failed), including failures. */
  judge_statuses?: PlayerJudgeStatusPayload[];
  /** The end-of-session debrief, once a report judge has written it. */
  session_report?: PlayerSessionReport | null;
  /** Open-ended evaluation state per open-ended task (criteria scores,
   *  TODO checklist, pending artifact requests, measurement history). */
  evaluations?: PlayerTaskEvaluation[];
  /** Copy/paste validation result from session finish, if the scan ran. */
  similarity_adjustment?: {
    note: string;
    point_delta: number;
    duplicated_pct?: number;
    sources?: { join_code: string; player: string; matched_lines: number }[];
  } | null;
}

/** One open-ended task's evaluation state on the player page. */
export interface PlayerTaskEvaluation {
  task_id: string;
  criteria: PlayerCriterionState[];
  /** Latest parsed TODO report: { checked, total, items: [{text, done}] }. */
  todo?: {
    checked: number;
    total: number;
    items: { text: string; done: boolean }[];
  } | null;
  pending_artifacts?: PlayerArtifactRequest[];
  deadline_at?: string | null;
  measurements?: PlayerMeasurementPoint[];
  artifacts?: PlayerArtifactRef[];
  /** Image files committed at the repo HEAD, for the gallery. */
  repo_images?: string[];
}

export interface PlayerArtifactRef {
  probe_id: string;
  content_type: string;
  label: string;
  /** Slug of the judge whose request this artifact answers. */
  judge_slug?: string | null;
  /** Files the request delivered (up to 5); `?i=N` addresses them. */
  file_count?: number;
  /** Repo path of each delivered file, in `?i=N` order. Absent on rows
   *  written before the list was carried — fall back to `label`. */
  files?: string[];
}

export interface PlayerCriterionState {
  key: string;
  title: string;
  weight: number;
  scores?: PlayerCriterionScore[];
}

export interface PlayerCriterionScore {
  judge_slug: string;
  /** null = the judge could not assess this criterion. */
  score?: number | null;
  rationale: string;
}

export interface PlayerArtifactRequest {
  probe_id: string;
  instruction: string;
  path: string;
  deadline_at: string;
}

export interface PlayerMeasurementPoint {
  test_ordinal: number;
  label: string;
  at: string;
  outcome?: string | null;
  result_json: Record<string, unknown>;
}

export interface PlayerHistoryFileDiff {
  path: string;
  status: "added" | "modified" | "deleted" | string;
  patch: string;
}

export interface PlayerHistoryCommit {
  sha: string;
  author_name: string;
  author_email: string;
  author_time: string;
  message: string;
  files: PlayerHistoryFileDiff[];
}

export interface PlayerHistoryResponse {
  commits: PlayerHistoryCommit[];
}

export interface PlayerDeltaPayload {
  seq: number;
  task_id: string;
  result: PlayerTaskResult | null;
  scheduler_state: PlayerSchedulerState | null;
  score: number;
  rank: number;
}

export interface ProbeUpdatedPayload extends ProbeFields {
  seq: number;
  probe_id: string;
  score: number;
  rank: number;
  updated_at: string;
  next_probe_at: string | null;
}

export interface PlayerSessionCompletePayload {
  seq: number;
  player_id: string;
  score: number;
  rank: number;
}

export interface TaskRevealedPayload {
  seq: number;
  task: PlayerTaskSummaryEntry;
  total_tasks: number;
}

export interface ScoreRankUpdatedPayload {
  seq: number;
  score: number;
  rank: number;
}

export interface PlayerJudgeScoredPayload {
  task_id: string;
  judge_slug: string;
  judge_name: string;
  rating: number;
  feedback: string;
  point_delta: number;
  created_at: string;
  /** Wall-clock duration of the judge run in milliseconds, when recorded. */
  duration_ms?: number | null;
}

/** Lifecycle state of one judge attachment for one task. */
/** One narrative written for the player at the end of the session. Carries no
 *  score: it is the debrief, not a verdict. */
export interface PlayerSessionReport {
  judge_name: string;
  judge_slug: string;
  /** The raw report. Prose only when the model failed to answer with the
   *  document; prefer `document` when it is present. */
  markdown: string;
  /** The structured report the page lays out. Absent for prose fallbacks. */
  document?: SessionReportDoc | null;
  created_at: string;
}

/** The report the page renders: a brief, the tasks it produced, the friction,
 *  what each judge thought, and what to do next. */
export interface SessionReportDoc {
  built: { brief: string; tasks?: { ordinal: number; note: string }[] };
  friction?: { ordinal: number; what_happened: string; why?: string | null }[];
  judges?: { judge: string; good: string; improve?: string | null }[];
  /** The reporter's word on each criterion across the whole session, keyed by
      criterion key. Absent on reports written before the reporter was asked
      for it — the scorecard then falls back to the panel's own last note. */
  criteria?: { key: string; summary: string }[];
  improve?: string[];
}

export interface PlayerJudgeStatusPayload {
  task_id: string;
  judge_slug: string;
  judge_name: string;
  status: "pending" | "running" | "scored" | "failed" | string;
  /** Generic public failure message; full detail is admin-only. */
  error?: string | null;
  updated_at?: string | null;
  /** Backing judge_results row, for the admin details endpoint. */
  judge_result_id?: string | null;
}

/** One chronological event of a judge run (admin details). Mirrors the
 * OpenTelemetry GenAI conventions rig emits: prompt/completion text,
 * per-turn transcript, token usage incl. provider cache. */
export interface JudgeLogEvent {
  at_ms: number;
  kind: "llm" | "tool" | string;
  name?: string | null;
  args?: string | null;
  output_chars?: number | null;
  duration_ms: number;
  tokens_input?: number | null;
  tokens_output?: number | null;
  tokens_cache_read?: number | null;
  tokens_cache_write?: number | null;
  /** Requested model id (gen_ai.request.model). */
  model?: string | null;
  /** System instructions + user prompt (gen_ai.prompt), truncated. */
  input?: string | null;
  /** Final completion text / tool result (gen_ai.completion), truncated. */
  output?: string | null;
  /** Full turn-by-turn agent transcript (assistant messages incl. tool
   * calls, tool results). */
  messages?: unknown;
  error?: string | null;
}

/** Full judge-run detail — admin-only endpoint. */
export interface JudgeResultDetails {
  id: string;
  task_judge_id: string;
  judge_slug?: string | null;
  judge_name?: string | null;
  status: string;
  model: string;
  provider: string;
  rating: unknown;
  point_delta: number;
  feedback: string;
  raw_output: string;
  duration_ms?: number | null;
  tokens_input?: number | null;
  tokens_output?: number | null;
  run_log?: JudgeLogEvent[] | null;
  /** The evidence snapshot the verdict was reached from. */
  evidence?: unknown;
  /** Fingerprint of the judge definition that produced this verdict — the
   * prompt behind a slug is re-seeded on every boot. */
  judge_fingerprint?: string | null;
  error?: string | null;
  created_at: string;
  updated_at: string;
}

/** One AI-agent session active during a task's implementation window. */
export interface AgentSessionStats {
  agent: string;
  agent_session_id: string;
  model?: string | null;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  reasoning_tokens: number;
  cost?: number | null;
  user_messages: number;
  assistant_messages: number;
  tool_calls: number;
  tools: Record<string, number>;
  skills: Record<string, number>;
}

/** Stored per-task agent statistics (client-reported by ololo). */
export interface TaskStatsEntry {
  task_id?: string | null;
  task_ordinal: number;
  window_started_at?: string | null;
  window_ended_at?: string | null;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  reasoning_tokens: number;
  cost?: number | null;
  user_messages: number;
  assistant_messages: number;
  tool_calls: number;
  agents: AgentSessionStats[];
  created_at: string;
  updated_at: string;
}

export interface TaskStatsResponse {
  entries: TaskStatsEntry[];
}

export type PlayerFrame =
  | ({ type: "player_snapshot" } & PlayerSnapshotPayload)
  | ({ type: "player_task_delta" } & PlayerDeltaPayload)
  | ({ type: "probe_updated" } & ProbeUpdatedPayload)
  | ({ type: "session_complete" } & PlayerSessionCompletePayload)
  | ({ type: "task_revealed" } & TaskRevealedPayload)
  | ({ type: "score_rank_updated" } & ScoreRankUpdatedPayload)
  | { type: "agent_presence"; connected: boolean; timestamp: string }
  | ({ type: "judge_scored" } & PlayerJudgeScoredPayload)
  | ({ type: "judge_started" } & PlayerJudgeStatusPayload)
  | ({ type: "judge_failed" } & PlayerJudgeStatusPayload)
  | {
      type: "session_status_change";
      status: "lobby" | "running" | "paused" | "finished" | "cancelled";
      cancel_reason?: string;
      cancelled_by?: string;
    }
  | {
      type: "artifact_awaited";
      task_id: string;
      probe_id: string;
      instruction: string;
      deadline_at: string;
    }
  | { type: "evaluation_ready"; task_id: string }
  /** The session report has been written — its text is in the snapshot. */
  | { type: "session_report_ready" }
  | { type: "player_error"; seq: number; message: string };

export interface TaskStartedPayload {
  player_id: string;
  player_display_name: string;
  task_id: string;
  task_ordinal: number;
  task_title: string;
  timestamp: string;
  version: number;
}

export interface TaskScoredPayload {
  player_id: string;
  player_display_name: string;
  task_id: string;
  task_ordinal: number;
  task_title: string;
  point_delta: number;
  judge_name: string;
  /** Criteria-judge verdicts: per-criterion sheet summary. */
  detail?: ActivityVerdictDetail | null;
  timestamp: string;
  version: number;
}

/** Compact criteria breakdown attached to a judge's activity entry. */
export interface ActivityVerdictDetail {
  overall?: number | null;
  criteria: { key: string; score: number | null; rationale?: string | null }[];
  /** The judge's written comment for this verdict, when it left one. */
  feedback?: string | null;
}

/** One file of a delivered artifact. */
export interface ArtifactFileInfo {
  path: string;
  size: number;
}

/** A delivered interactive-probe artifact attached to an activity entry. */
export interface ActivityArtifactDetail {
  probe_id: string;
  /** Repo-relative path of the first delivered file. */
  path: string;
  size: number;
  content_type: string;
  within_cap?: boolean;
  /** Every delivered file (up to 5); absent on entries recorded before
   *  multi-file requests existed. */
  files?: ArtifactFileInfo[];
}

export interface ActivitySimilarityDetail {
  duplicated_pct: number;
  duplicated_lines: number;
  total_lines: number;
  corpus_repos: number;
  sources: { join_code: string; player: string; matched_lines: number }[];
}

export type ActivityDetail = Partial<ActivityVerdictDetail> &
  Partial<ActivityArtifactDetail> &
  Partial<ActivitySimilarityDetail>;

export interface ArtifactReceivedPayload {
  player_id: string;
  player_display_name: string;
  task_id: string;
  task_ordinal: number;
  task_title: string;
  probe_id: string;
  path: string;
  size: number;
  content_type: string;
  within_cap: boolean;
  files?: ArtifactFileInfo[];
  timestamp: string;
  version: number;
}

export interface ActivityEvent {
  kind: "task_started" | "task_scored" | "artifact_received" | "similarity";
  player_id: string;
  player_display_name: string;
  task_id: string;
  task_ordinal: number;
  task_title: string;
  judge_name: string | null;
  point_delta: number | null;
  detail?: ActivityDetail | null;
  timestamp: string;
  version: number;
}

export type ArenaFrame =
  | { type: "test_push"; task_id: string; attempt: number; test_template: unknown }
  | { type: "session_complete"; reason: string; version?: number }
  | { type: "session_settled"; session_id: string; version?: number }
  | {
      type: "session_cancelled";
      session_id: string;
      version?: number;
      cancel_reason?: string;
      cancelled_by?: string;
    }
  | { type: "leaderboard_update"; entries: LeaderboardEntry[]; version?: number }
  | {
      type: "player_progress_update";
      player_id: string;
      current_task_id: string | null;
      attempt: number;
      status: AgentRunStatus;
    }
  // Backward-compat for older frame name.
  | {
      type: "agent_progress_update";
      agent_id: string;
      current_task_id: string | null;
      attempt: number;
      status: AgentRunStatus;
    }
  | {
      type: "test_result";
      task_id: string;
      attempt: number;
      pass: boolean;
      duration_ms: number;
      exit_code: number;
      stdout_tail: string;
    }
  | { type: "heartbeat" }
  | { type: "lobby_countdown"; session_id: string; seconds_remaining: number; version?: number }
  | { type: "running_countdown"; session_id: string; seconds_remaining: number; version?: number }
  | { type: "session_started"; session_id: string; version?: number }
  | { type: "member_list"; members: MemberInfo[] }
  | {
      type: "member_joined";
      user_id: string;
      player_id?: string | null;
      display_name: string;
      joined_at: string;
      avatar_url?: string | null;
      fingerprint?: string | null;
      username?: string | null;
      agent_display_name?: string | null;
      version?: number;
    }
  | ({ type: "session_snapshot" } & SessionSnapshotPayload)
  | { type: "player_disconnected"; player_id: string; version?: number }
  // Backward-compat for older frame name.
  | { type: "member_disconnected"; agent_id: string; version?: number }
  | {
      type: "project_session_update";
      session_id: string;
      name: string;
      status: string;
      project_id: string;
      join_code: string | null;
      created_at: string;
      /** Players currently in the session; re-sent on join/leave. */
      player_count?: number;
      cancel_reason?: string;
      cancelled_by?: string;
    }
  | { type: "admin_adapted_tasks_snapshot"; session_id: string; players: AdminAdaptedTaskView[] }
  | {
      type: "admin_adapted_task_updated";
      session_id: string;
      player_id: string;
      entry: AdminTaskEntry;
    }
  | { type: "user_players_snapshot"; players: PlayerSummary[] }
  | ({ type: "task_started" } & TaskStartedPayload)
  | ({ type: "task_scored" } & TaskScoredPayload)
  | ({ type: "artifact_received" } & ArtifactReceivedPayload);

export type ZmqEvent =
  | {
      type: "session_timer";
      join_code: string;
      phase: string;
      seconds_remaining: number;
      version: number;
    }
  | { type: "session_status"; join_code: string; status: string; version: number }
  | {
      type: "player_join";
      join_code: string;
      player_id: string;
      display_name: string;
      user_id: string | null;
      joined_at: string;
      avatar_url: string | null;
      fingerprint: string | null;
      username: string | null;
      version: number;
    }
  | { type: "player_leave"; join_code: string; player_id: string; version: number }
  | {
      type: "score_change";
      join_code: string;
      player_id: string;
      delta: number;
      total: number;
      version: number;
    };

export interface ToolEntry {
  name: string;
  version?: string | null;
}

export interface PlayerMetadataResponse {
  player_id: string;
  display_name: string;
  joined_at: string;
  fingerprint: string | null;
  ai_agents: ToolEntry[];
  build_tools: ToolEntry[];
  languages: ToolEntry[];
  test_tools: ToolEntry[];
  utility_tools: ToolEntry[];
  probe_duration_ms: number | null;
}

export type PlayerMetadataMap = Record<string, PlayerMetadataResponse>;

/** One session-memory key: schema default overlaid with the extracted value. */
export interface PlayerMemoryEntry {
  key: string;
  /** Current effective value (extracted when available, else the default). */
  value: string;
  default: string;
  /** True when the value was LLM-extracted from the player's markdown docs. */
  extracted: boolean;
}

export interface PlayerMemoryResponse {
  /** False when the project declares no memory schema (tab hidden). */
  enabled: boolean;
  entries: PlayerMemoryEntry[];
  updated_at: string | null;
}
