// Domain types for the Arena REST client. Mirrors server response/request shapes.
// Kept separate from function modules to avoid circular imports.

import type { LeaderboardEntry, ScoreHistoryPoint } from "$lib/types/arena";

export interface Session {
  id: string;
  name: string;
  status: string;
  owner_id: string | null;
  project_id: string;
  created_at: string;
  join_code?: string | null;
  /** Players in the session. Only the project listing populates this. */
  player_count?: number;
  /** Winner's name; project listing only, and only once finished. */
  best_player?: string | null;
  /** The winner's points. Paired with `best_player`. */
  best_score?: number | null;
}

export interface Member {
  session_id: string;
  user_id: string;
  role: string;
  joined_at: string;
}

// ---------- Points (project defaults + task overrides) ----------
export interface PointsResp {
  value: number;
  fail: number;
  no_response: number;
  completion_bonus: number;
}

export interface PointsReq {
  value?: number | null;
  fail?: number | null;
  no_response?: number | null;
  completion_bonus?: number | null;
}

export interface PointsRange {
  min: number;
  max: number;
}

// ---------- Intervals (project defaults + task overrides) ----------
export interface IntervalsReq {
  deadline_secs?: number | null;
  min_interval_secs?: number | null;
  interval_increment_secs?: number | null;
  max_interval_secs?: number | null;
}

export interface IntervalsResp {
  deadline_secs: number;
  min_interval_secs: number;
  interval_increment_secs: number;
  max_interval_secs: number;
}

export interface ExportIntervals {
  deadline_secs: number;
  min_interval_secs: number;
  interval_increment_secs: number;
  max_interval_secs: number;
}

export interface ExportTaskIntervals {
  deadline_secs?: number | null;
  min_interval_secs?: number | null;
  interval_increment_secs?: number | null;
  max_interval_secs?: number | null;
}

export type TestKind = "shell" | "http_request" | "file_exists";

export interface Placeholder {
  name: string;
  description: string;
  required?: boolean;
  secret?: boolean;
}

export interface Matchers {
  expected_exit_code: number;
  stdout_contains?: string;
  stdout_regex?: string;
  max_duration_ms: number;
  param_timeout_ms: number;
}

export interface Backoff {
  initial_ms: number;
  multiplier: number;
  max_ms: number;
  max_attempts: number;
}

export interface TestTemplate {
  kind: TestKind;
  command_template: string;
  answer_template?: string;
  fixtures?: unknown[];
  placeholders?: Placeholder[];
  matchers?: Matchers;
  backoff?: Backoff;
}

export interface Task {
  id: string;
  project_id: string;
  ordinal: number;
  title: string;
  description: string;
  tags: string[];
  test_template: TestTemplate;
  created_at: string;
  points: PointsResp;
  intervals: IntervalsResp;
}

export interface CreateTaskBody {
  title: string;
  description?: string;
  ordinal?: number;
  tags?: string[];
  test_template: TestTemplate;
  points?: PointsReq;
  intervals?: IntervalsReq;
}

export interface Agent {
  id: string;
  session_id: string;
  user_id: string;
  kind: string;
  model: string;
  display_name: string;
  registered_at: string;
  revoked_at: string | null;
}

export interface RegisterAgentBody {
  kind: "claude-code" | "opencode" | "custom";
  model: string;
  display_name: string;
}

export interface Project {
  id: string;
  name: string;
  owner_user_id: string;
  public: boolean;
  archived_at: string | null;
  created_at: string;
  updated_at: string;
  description: string;
  slug: string | null;
  has_active_sessions: boolean;
  category: string | null;
  tags: string[];
  cover_image_url: string | null;
  task_count: number;
  points: PointsResp;
  points_range: PointsRange | null;
  intervals: IntervalsResp;
  session_duration_secs: number;
  /** Cancel a running session after this many idle seconds; 0 disables. */
  idle_timeout_secs: number;
  /** Session-memory schema: flat object of key -> scalar default, or null. */
  memory_schema?: Record<string, string | number | boolean> | null;
  /** Whether the public project page lists the task arc up front. */
  show_tasks: boolean;
  /** Finished sessions played on this project; list endpoint only. */
  session_count?: number;
  /** Finished sessions the signed-in caller has played here; list endpoint
   *  only, and only when authenticated (the per-user "played" indicator). */
  user_session_count?: number;
  /** Estimated judge reviews a full session triggers (total judges across all tasks). */
  judge_review_count?: number;
  /** Campaign this project is a part of, and its position in it. */
  parent_project_id?: string | null;
  part_ordinal?: number | null;
  /** Parts this project has as a campaign parent; > 0 means it is a campaign. */
  part_count?: number;
  /** A campaign's playing time: its parts' durations added up. A campaign's
   *  own session_duration_secs is a default nobody plays. */
  parts_duration_secs?: number;
  parent_project_slug?: string | null;
  parent_project_name?: string | null;
}

/** Per-caller progression state of one campaign part. */
export type ProjectPartState = "locked" | "available" | "in_progress" | "completed";

/** One part of a campaign: the same project summary every other listing
 *  carries — so the campaign page renders the catalog's card — plus the
 *  caller's progression on it. */
export interface ProjectPart extends Project {
  state: ProjectPartState;
}

/** A judge attached to a previewed task — name and face only. */
export interface TaskPreviewJudge {
  slug: string;
  name: string;
  /** Public blurb for the hover card; absent on pre-upgrade servers. */
  description?: string;
  avatar_url?: string | null;
}

/** One row of the public task-arc preview (title + brief only — never the
 * grading machinery). */
export interface TaskPreviewItem {
  ordinal: number;
  title: string;
  description: string;
  /** Classic task: award per passing check. Open-ended: the judge panel's point budget. */
  points: number;
  /** Absent on pre-upgrade servers. */
  open_ended?: boolean;
  completion_bonus?: number;
  judges?: TaskPreviewJudge[];
}

export interface ProjectTask {
  id: string;
  project_id: string;
  ordinal: number;
  title: string;
  description: string;
  tags: string[];
  test_template: TestTemplate;
  created_at: string;
  result_count: number;
  session_count: number;
  points: PointsResp;
  intervals: IntervalsResp;
}

export interface CreateProjectRequest {
  name: string;
  public?: boolean;
  description?: string;
  category?: string;
  tags?: string[];
  cover_image_url?: string;
  points?: PointsReq;
  intervals?: IntervalsReq;
  session_duration_secs?: number;
  idle_timeout_secs?: number;
  memory_schema?: Record<string, string | number | boolean>;
}

export interface PatchProjectRequest {
  name?: string;
  public?: boolean;
  archived?: boolean;
  description?: string;
  slug?: string;
  category?: string;
  tags?: string[];
  cover_image_url?: string;
  clear_cover_image?: boolean;
  points?: PointsReq;
  intervals?: IntervalsReq;
  session_duration_secs?: number;
  idle_timeout_secs?: number;
  /** Session-memory schema. Explicit `null` clears it; absent leaves it as is. */
  memory_schema?: Record<string, string | number | boolean> | null;
}

export interface TaskInput {
  title: string;
  description?: string;
  ordinal?: number;
  tags?: string[];
  test_template: TestTemplate;
  points?: PointsReq;
  intervals?: IntervalsReq;
}

export interface TaskDraft {
  title: string;
  description: string;
  command_template?: string;
  tags?: string[];
}

// ---------- Auth ----------
export interface RegisterBody {
  email: string;
  password: string;
  display_name: string;
  turnstile_token?: string;
}

export interface LoginBody {
  email: string;
  password: string;
  turnstile_token?: string;
}

// ---------- Current User ----------
export interface PatDto {
  id: string;
  fingerprint: string;
  created_at: string;
  expires_at: string | null;
}

export interface Me {
  id: string;
  email: string;
  display_name: string;
  is_admin: boolean;
  avatar_url: string | null;
  allow_project_creation: boolean;
  email_verified: boolean;
  username: string;
  /** Account plan: "free" or "premium". */
  plan: string;
  /** Metered judge runs charged to this account this calendar month. */
  judge_runs_used: number;
  /** This month's judge-run limit (per-user override or the plan's tier limit). */
  judge_run_limit: number;
  /** Purchased pack credits still unspent (consumed after the monthly limit). */
  judge_run_credits: number;
  /** Whether judge-run quotas are enforced on this instance. */
  plans_enabled: boolean;
  /** Whether this instance offers the session replay bar (admins only see it
   *  either way; this is the instance-wide switch). */
  session_replay_enabled?: boolean;
}

export interface PublicUserProfile {
  username: string;
  display_name: string;
  avatar_url: string | null;
  /** Account creation timestamp (RFC 3339). */
  joined_at: string;
}

export interface PublicSessionEntry {
  session_id: string;
  name: string;
  session_datetime: string;
  participant_count: number;
  status: string;
  join_code: string;
  project_id: string;
  project_name: string;
  project_slug: string | null;
  /** Raw in-game score (task + judge point deltas); can be negative. */
  game_points: number;
  /** Final place on this session's leaderboard; null until awarded. */
  placement: number | null;
  /** Coding agent used, e.g. "claude"; null when never reported. */
  agent: string | null;
  /** Models observed in client-reported stats. */
  models: string[];
}

export interface PublicSessionsResponse {
  sessions: PublicSessionEntry[];
  total: number;
  page: number;
  per_page: number;
}

export interface SessionReportTimelineEntry {
  task_id: string;
  task_title: string;
  player_id: string;
  player_display_name: string;
  score: number;
  answer: string;
  created_at: string;
}

export interface ActivityEventDto {
  event_kind: string;
  player_id: string;
  player_display_name: string;
  task_id: string;
  task_ordinal: number;
  task_title: string;
  judge_name: string | null;
  point_delta: number | null;
  /** Criteria-judge verdicts: `{overall, criteria: [{key, score}]}`. */
  detail?: import("$lib/types/arena").ActivityVerdictDetail | null;
  timestamp: string;
  version: number;
}

export interface ReportPlayerDto {
  player_id: string;
  /** The user who owns this player; null for anonymous/unlinked players. */
  user_id: string | null;
  display_name: string;
  avatar_url: string | null;
  username: string | null;
}

export interface SessionReportResponse {
  session_id: string;
  status: string;
  leaderboard: LeaderboardEntry[];
  timeline: SessionReportTimelineEntry[];
  activity_events: ActivityEventDto[];
  /** Optional: absent in payloads produced before the field was added. */
  players?: ReportPlayerDto[];
  /** Optional: absent in older payloads; null when no scores were recorded. */
  score_history?: ScoreHistoryPoint[] | null;
  /** Judge runs still owed after the finish; absent on older servers. */
  judges_pending?: number;
}

/** Per-player statistics for the session page Statistics block. */
export interface SessionPlayerStats {
  player_id: string;
  user_id: string | null;
  display_name: string;
  avatar_url: string | null;
  username: string | null;
  agent_display_name: string | null;
  rank: number;
  game_points: number;
  probe_points: number;
  bonus_points: number;
  judge_points: number;
  solved_tasks: number;
  probes: number;
  agents: string[];
  models: string[];
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  reasoning_tokens: number;
  cost: number | null;
  tool_calls: number;
}

export interface SessionPlayerStatsResponse {
  total_tasks: number;
  players: SessionPlayerStats[];
}

export interface AvatarAuthResponse {
  token: string;
  expire: number;
  signature: string;
  public_key: string;
  url_endpoint: string;
}

// ---------- Admin Settings ----------
/** Flat key-value app settings; AI model config lives in /api/admin/llm/*. */
export type AppSettings = Record<string, string>;

// ---------- Categories ----------
export interface CategoryDto {
  id: number;
  name: string;
  /** Projects currently filed under this category name. */
  project_count?: number;
}

// ---------- Admin Users ----------
export interface AdminUserDto {
  id: string;
  email: string;
  display_name: string;
  username: string | null;
  is_admin: boolean;
  avatar_url: string | null;
  created_at: string;
  /** Account plan: "free" or "premium". */
  plan: string;
  /** Per-user monthly judge-run limit override; null = tier limit applies. */
  judge_run_limit: number | null;
  /** The limit actually in force (override, else the plan's tier limit). */
  judge_run_limit_effective: number;
  /** Metered judge runs charged to this account this calendar month. */
  judge_runs_this_month: number;
  /** Purchased pack credits still unspent. */
  judge_run_credits: number;
}

export interface CreateAdminUserBody {
  email: string;
  display_name: string;
  username?: string;
  password: string;
  is_admin: boolean;
  /** Account plan; omitted → "premium". */
  plan?: string;
}

export interface UpdateAdminUserBody {
  email?: string;
  display_name?: string;
  username?: string;
  is_admin?: boolean;
  password?: string;
  /** Account plan: "free" or "premium". */
  plan?: string;
  /** null clears the per-user override; a number sets it. */
  judge_run_limit?: number | null;
  /** Delta added to the purchased-credits balance (a pack sold). */
  grant_judge_run_credits?: number;
}

// ---------- Admin Game Servers ----------
export interface GameServerDto {
  id: string;
  url: string;
  zmq_url: string | null;
  display_name: string | null;
  capacity: number;
  active_sessions: number;
  status: string;
  last_heartbeat: string;
  created_at: string;
  updated_at: string;
}

// ---------- Email / Password Reset / Magic Link ----------
export interface TurnstileConfigResponse {
  enabled: boolean;
  sitekey: string | null;
}

// ---------- Email Templates ----------
export interface EmailTemplate {
  type: string;
  subject: string;
  body_html: string;
  body_text: string;
  updated_at: string;
}

// ---------- Project Export / Import ----------
export interface ExportPoints {
  value: number;
  fail: number;
  no_response: number;
  completion_bonus: number;
}

export interface ExportTaskPoints {
  value?: number | null;
  fail?: number | null;
  no_response?: number | null;
  completion_bonus?: number | null;
}

export interface ExportTask {
  ordinal: number;
  title: string;
  content: string;
  test_template: any;
  tags: string[];
  points?: ExportTaskPoints | null;
  intervals?: ExportTaskIntervals;
}

export interface ExportProject {
  name: string;
  slug: string | null;
  description: string | null;
  category: string | null;
  tags: string[];
  cover_image_url: string | null;
  public: boolean;
  archived_at: string | null;
  points: ExportPoints;
  intervals: ExportIntervals;
}

export interface ExportEnvelope {
  schema_version: number;
  project: ExportProject;
  tasks: ExportTask[];
}

// ---------- Judges ----------
export interface RatingScale {
  min: number;
  max: number;
  step: number;
}

export interface Judge {
  id: string;
  slug: string;
  name: string;
  description: string;
  prompt: string;
  rating_scale: RatingScale;
  /** Per-judge LLM provider override; null inherits the operation/default assignment. */
  llm_provider_id: string | null;
  /** Per-judge model override; null inherits the operation/default assignment. */
  llm_model: string | null;
  /** Per-judge pool override; may be combined with the provider+model pair. */
  llm_pool_id: string | null;
  /** Which override half leads when both are set. */
  llm_source_order: LlmSourceOrder;
  /** Optional avatar shown beside the judge's verdicts. */
  avatar_url: string | null;
  created_at: string;
  updated_at: string;
}

/** One task a judge is attached to, with the project that owns it. */
export interface JudgeAttachment {
  project_id: string;
  project_name: string;
  project_slug: string | null;
  /** Set when the project is a campaign part. */
  parent_project_id: string | null;
  task_id: string;
  task_ordinal: number;
  task_title: string;
}

/** What a judge has done, across every session it ever ran in. */
export interface JudgeStats {
  /** Verdicts that landed. */
  verdicts: number;
  /** Runs that ended without one. */
  failed_runs: number;
  /** Net points moved: awarded minus withdrawn. */
  points_total: number;
  points_awarded: number;
  points_withdrawn: number;
  sessions: number;
  players: number;
  last_verdict_at: string | null;
}

export interface JudgeUsage {
  judge_id: string;
  attachments: JudgeAttachment[];
  stats: JudgeStats;
}

/**
 * Order the two halves of a judge's override are tried in. `pool_first`
 * (the default) runs the pool's candidates before the pinned model;
 * `model_first` is the reverse. The half that is not first becomes the
 * failover behind the one that is.
 */
export type LlmSourceOrder = "pool_first" | "model_first";

export interface CreateJudgeBody {
  slug: string;
  name: string;
  description: string;
  prompt: string;
  rating_scale: RatingScale;
  /**
   * Optional per-judge model override (llm_providers row + model id). The
   * two travel together — sending one without the other is a 400.
   */
  llm_provider_id?: string | null;
  llm_model?: string | null;
  /** Optional per-judge pool override; composes with the pair above. */
  llm_pool_id?: string | null;
  /** Defaults to `pool_first` when omitted. */
  llm_source_order?: LlmSourceOrder;
  /** Optional avatar (https URL on the configured image host). */
  avatar_url?: string | null;
}

export interface UpdateJudgeBody {
  name?: string;
  description?: string;
  prompt?: string;
  rating_scale?: RatingScale;
  /**
   * Absent = unchanged; explicit null clears. The provider and the model
   * are validated as a pair against the state the patch lands on, so
   * clearing one alone is a 400 — send both.
   */
  llm_provider_id?: string | null;
  llm_model?: string | null;
  /** Absent = unchanged; explicit null clears the pool override. */
  llm_pool_id?: string | null;
  llm_source_order?: LlmSourceOrder;
  /** Absent = unchanged; explicit null clears the avatar. */
  avatar_url?: string | null;
}

export interface TaskJudge {
  id: string;
  task_id: string;
  judge_id: string;
  judge_slug: string;
  judge_name: string;
  ordinal: number;
  rating_scale_override: RatingScale | null;
  effective_rating_scale: RatingScale;
  created_at: string;
  updated_at: string;
}

export interface AttachTaskJudgeBody {
  judge_id: string;
  ordinal: number;
  rating_scale_override?: RatingScale;
}

export interface UpdateTaskJudgeBody {
  ordinal?: number;
  rating_scale_override?: RatingScale | null;
}

// ---------- Judge Runs ----------
export interface JudgeRunRequest {
  task_id: string;
  judge_id: string;
  /** Overwrite a verdict that already exists instead of returning it. */
  force?: boolean;
}

export interface JudgeRunResult {
  judge_result_id: string;
  rating: number;
  feedback: string;
  point_delta: number;
  raw_output: string;
  model: string;
}

/** One row of the instance-wide session registry in admin settings. */
export interface AdminSession {
  id: string;
  join_code: string;
  name: string;
  status: string;
  project_id: string;
  /** Null only if the project was removed out from under the session. */
  project_name: string | null;
  project_slug: string | null;
  owner_id: string | null;
  owner_display_name: string | null;
  owner_username: string | null;
  /** Players who have not been revoked. */
  player_count: number;
  created_at: string;
  started_at: string | null;
  finished_at: string | null;
  /** "user" or "idle_timeout" for cancelled sessions. */
  cancel_reason: string | null;
  cancelled_by: string | null;
}

export interface AdminSessionsResponse {
  sessions: AdminSession[];
  total: number;
  page: number;
  per_page: number;
}

/** Filters the registry accepts; every field is optional. */
export interface AdminSessionsQuery {
  page?: number;
  per_page?: number;
  status?: string;
  project_id?: string;
  /** Matches a join code or a session name. */
  q?: string;
}
