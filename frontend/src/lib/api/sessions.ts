// Sessions, members, join codes, session tasks and agents.

import { request, type FetchLike } from "./core";
import type {
  Session,
  Member,
  Task,
  Agent,
  RegisterAgentBody,
  AdminSessionsResponse,
  AdminSessionsQuery,
} from "./types";

/** One live (lobby/running) session of a public project, as the landing shows it. */
export interface PublicActiveSession {
  /** Session DB id — the key `/ws/landing/observe` frames use. */
  id: string;
  join_code: string;
  name: string;
  status: "lobby" | "running";
  project_name: string;
  project_slug: string | null;
  cover_image_url: string | null;
  players: number;
  created_at: string;
  started_at: string | null;
  /** Server-time countdown snapshot: lobby → until autostart, running → until
   * the session ends. Ticked down client-side; null when not computable. */
  seconds_remaining: number | null;
}

/**
 * Public list of live sessions for the landing page — no auth required.
 * Lobby (joinable) sessions come first, newest first within each group.
 */
export async function listActiveSessions(
  opts: { fetch?: FetchLike } = {},
): Promise<PublicActiveSession[]> {
  const data = await request<{ sessions: PublicActiveSession[] }>("/api/public/active-sessions", {
    fetch: opts.fetch,
  });
  return data.sessions;
}

export async function listSessions(opts: { fetch?: FetchLike } = {}): Promise<Session[]> {
  const data = await request<{ sessions: Session[] }>("/api/sessions", {
    fetch: opts.fetch,
  });
  return data.sessions;
}

/**
 * Every session on the instance (admin only), newest first.
 *
 * Distinct from {@link listSessions}, which answers with the caller's own
 * sessions and so cannot reach a session started by somebody else.
 */
export function listAdminSessions(
  query: AdminSessionsQuery = {},
  opts: { fetch?: FetchLike } = {},
): Promise<AdminSessionsResponse> {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) {
    // An empty filter must be absent, not sent as "": the server rejects an
    // unparseable status rather than ignoring it.
    if (value !== undefined && value !== null && value !== "") params.set(key, String(value));
  }
  const qs = params.toString();
  return request<AdminSessionsResponse>(`/api/admin/sessions${qs ? `?${qs}` : ""}`, {
    fetch: opts.fetch,
  });
}

/** `host` marks a session run for a group from the web — the caller holds
 *  no player row in it, so the server lets them have several at once. */
export function createSession(
  name: string,
  project_id: string,
  opts: {
    fetch?: FetchLike;
    host?: boolean;
    /** The join code of a session to link the new one with (one event). */
    link_with?: string;
  } = {},
) {
  const body: Record<string, unknown> = { name, project_id };
  if (opts.host) body.host = true;
  if (opts.link_with) body.link_with = opts.link_with;
  return request<Session>("/api/sessions", {
    method: "POST",
    body: opts.host ? { name, project_id, host: true } : { name, project_id },
    fetch: opts.fetch,
  });
}

export function getSession(id: string, opts: { fetch?: FetchLike } = {}) {
  return request<Session>(`/api/sessions/${encodeURIComponent(id)}`, {
    fetch: opts.fetch,
  });
}

/** Public lookup — no auth required. Returns minimal session info with project link. */
export function getSessionByCode(code: string, opts: { fetch?: FetchLike } = {}) {
  return request<{
    id: string;
    join_code: string;
    state: string;
    owner_id: string | null;
    project_id: string | null;
    project_name: string | null;
    project_slug: string | null;
    project_description: string | null;
    /** Whether signed-in spectators may open a player's run page — mirrors
        the snapshot endpoint's visibility rule. Absent on older servers. */
    project_public?: boolean;
    created_at?: string;
    started_at?: string | null;
    finished_at?: string | null;
  }>(`/api/sessions/by-code/${encodeURIComponent(code)}`, { fetch: opts.fetch });
}

/** One player of this session who already cleared a campaign part. */
export interface CampaignClearedBy {
  user_id: string;
  display_name: string;
  /** The session that part was cleared in. */
  join_code: string;
  finished_at: string | null;
}

export interface SessionCampaignPart {
  project_id: string;
  name: string;
  slug: string | null;
  part_ordinal: number;
  /** The part this session is playing. */
  current: boolean;
  /**
   * Players of this session who cleared the part. For earlier parts that is
   * their own past run, and `join_code` links to it; for the current part it
   * is this session, answered by this session alone.
   */
  cleared_by: CampaignClearedBy[];
}

export interface SessionCampaign {
  id: string;
  name: string;
  slug: string | null;
  current_part_ordinal: number;
  /** The session's own status — "playing now" is only true while it runs. */
  session_status: string;
  parts: SessionCampaignPart[];
}

/**
 * Campaign context of a session: the ladder and which earlier parts the
 * session's players already cleared. `null` when the session is not part of a
 * campaign (the endpoint answers 204). Public, like the by-code lookup.
 */
export async function getSessionCampaign(
  code: string,
  opts: { fetch?: FetchLike } = {},
): Promise<SessionCampaign | null> {
  const data = await request<SessionCampaign | undefined>(
    `/api/sessions/by-code/${encodeURIComponent(code)}/campaign`,
    { fetch: opts.fetch },
  );
  return data ?? null;
}

/** List all sessions for a project. Requires auth; project owners see all sessions. */
export async function listProjectSessions(
  projectId: string,
  opts: { fetch?: FetchLike } = {},
): Promise<Session[]> {
  const resp = await request<{ sessions: Session[] }>(
    `/api/projects/${encodeURIComponent(projectId)}/sessions`,
    { fetch: opts.fetch },
  );
  return resp.sessions;
}

export function patchSession(
  id: string,
  patch: Partial<{ name: string; status: string }>,
  opts: { fetch?: FetchLike } = {},
) {
  return request<Session>(`/api/sessions/${encodeURIComponent(id)}`, {
    method: "PATCH",
    body: patch,
    fetch: opts.fetch,
  });
}

export function deleteSession(id: string, opts: { fetch?: FetchLike } = {}) {
  return request<void>(`/api/sessions/${encodeURIComponent(id)}`, {
    method: "DELETE",
    fetch: opts.fetch,
  });
}

export async function listMembers(id: string, opts: { fetch?: FetchLike } = {}): Promise<Member[]> {
  const data = await request<{ members: Member[] }>(
    `/api/sessions/${encodeURIComponent(id)}/members`,
    { fetch: opts.fetch },
  );
  return data.members;
}

export function addMember(
  id: string,
  user_id: string,
  role: "participant" | "observer",
  opts: { fetch?: FetchLike } = {},
) {
  return request<Member>(`/api/sessions/${encodeURIComponent(id)}/members`, {
    method: "POST",
    body: { user_id, role },
    fetch: opts.fetch,
  });
}

export function removeMember(id: string, user_id: string, opts: { fetch?: FetchLike } = {}) {
  return request<void>(
    `/api/sessions/${encodeURIComponent(id)}/members/${encodeURIComponent(user_id)}`,
    { method: "DELETE", fetch: opts.fetch },
  );
}

export function joinSession(code: string, opts: { fetch?: FetchLike } = {}) {
  return request<{ session_id: string }>("/api/sessions/join", {
    method: "POST",
    body: { code },
    fetch: opts.fetch,
  });
}

export function regenerateJoinCode(sessionId: string, opts: { fetch?: FetchLike } = {}) {
  return request<{ join_code: string }>(
    `/api/sessions/${encodeURIComponent(sessionId)}/regenerate-code`,
    { method: "POST", fetch: opts.fetch },
  );
}

export async function listTasks(id: string, opts: { fetch?: FetchLike } = {}): Promise<Task[]> {
  const data = await request<{ tasks: Task[] }>(`/api/sessions/${encodeURIComponent(id)}/tasks`, {
    fetch: opts.fetch,
  });
  return data.tasks;
}

export async function listAgents(id: string, opts: { fetch?: FetchLike } = {}): Promise<Agent[]> {
  const data = await request<{ agents: Agent[] }>(
    `/api/sessions/${encodeURIComponent(id)}/agents`,
    { fetch: opts.fetch },
  );
  return data.agents;
}

export function registerAgent(
  id: string,
  body: RegisterAgentBody,
  opts: { fetch?: FetchLike } = {},
) {
  return request<{ agent: Agent; observer_token: string }>(
    `/api/sessions/${encodeURIComponent(id)}/agents`,
    { method: "POST", body, fetch: opts.fetch },
  );
}

/** Admin-only: a player's full session event log as raw JSONL (task starts,
 * probes, score changes, judge runs with full telemetry, session status). */
export async function getPlayerEventLog(
  sessionCode: string,
  playerId: string,
  opts: { fetch?: FetchLike } = {},
): Promise<string> {
  const raw = await request<unknown>(
    `/api/admin/sessions/${encodeURIComponent(sessionCode)}/players/${encodeURIComponent(playerId)}/event-log`,
    { fetch: opts.fetch },
  );
  // `request` JSON-parses when it can: a multi-line JSONL body comes back as
  // the raw string, but a single-line log parses into an object. Undo that.
  return typeof raw === "string" ? raw : JSON.stringify(raw);
}

// ---------- Linked sessions ----------

/** One session of a linked group, as the pages name it. */
export interface LinkedSession {
  id: string;
  join_code: string;
  name: string;
  status: string;
  project_id: string;
  project_name: string | null;
  project_slug: string | null;
  created_at: string;
  started_at: string | null;
  finished_at: string | null;
  player_count: number;
  /** The session the page is about. */
  current: boolean;
}

/** What one session contributed to a combined row. */
export interface CombinedSessionPoints {
  session_id: string;
  points: number;
  /** The player's place in that session, 1-based. */
  rank: number;
}

/** One row of the combined standings across linked sessions — a player's
 *  points added up by account. */
export interface CombinedEntry {
  user_id: string | null;
  display_name: string;
  username?: string | null;
  avatar_url?: string | null;
  total_points: number;
  sessions_played: number;
  per_session: CombinedSessionPoints[];
}

export interface SessionLinks {
  group_id: string;
  /** In the order they were linked. */
  sessions: LinkedSession[];
  /** Every session in the group has ended — the combined standings are final. */
  all_finished: boolean;
  leaderboard: CombinedEntry[];
}

/**
 * The group a session is linked into — the sessions and the combined
 * standings across them. `null` when it is linked to nothing (the endpoint
 * answers 204). Public, like the by-code lookup.
 */
export async function getSessionLinks(
  code: string,
  opts: { fetch?: FetchLike } = {},
): Promise<SessionLinks | null> {
  const data = await request<SessionLinks | undefined>(
    `/api/sessions/by-code/${encodeURIComponent(code)}/links`,
    { fetch: opts.fetch },
  );
  return data ?? null;
}

/** Link session `id` with the one `joinCode` names. Host of both, or admin.
 *  Answers with the group. */
export function linkSession(id: string, joinCode: string, opts: { fetch?: FetchLike } = {}) {
  return request<SessionLinks>(`/api/sessions/${encodeURIComponent(id)}/links`, {
    method: "POST",
    body: { join_code: joinCode },
    fetch: opts.fetch,
  });
}

/** Take session `id` out of its group. Host or admin. */
export function unlinkSession(id: string, opts: { fetch?: FetchLike } = {}) {
  return request<void>(`/api/sessions/${encodeURIComponent(id)}/links`, {
    method: "DELETE",
    fetch: opts.fetch,
  });
}
