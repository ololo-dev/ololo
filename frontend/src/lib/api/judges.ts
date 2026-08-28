// Judges, task-judge attachments, and judge runs.

import { request, type FetchLike } from "./core";
import type {
  Judge,
  CreateJudgeBody,
  UpdateJudgeBody,
  TaskJudge,
  AttachTaskJudgeBody,
  UpdateTaskJudgeBody,
  JudgeRunRequest,
  JudgeRunResult,
  JudgeUsage,
} from "./types";

export function listJudges(opts: { fetch?: FetchLike } = {}): Promise<Judge[]> {
  return request<Judge[]>("/api/admin/judges", { fetch: opts.fetch });
}

export interface SyncJudgesResponse {
  inserted: number;
  updated: number;
  skipped: number;
}

/** Re-run the judge seed against the server's on-disk judges/*.md (admin only). */
export function syncJudges(opts: { fetch?: FetchLike } = {}): Promise<SyncJudgesResponse> {
  return request<SyncJudgesResponse>("/api/admin/judges/sync", {
    method: "POST",
    fetch: opts.fetch,
  });
}

/** Where each judge is attached, and what it has done (admin only). */
export function listJudgeUsage(opts: { fetch?: FetchLike } = {}): Promise<JudgeUsage[]> {
  return request<JudgeUsage[]>("/api/admin/judges/usage", { fetch: opts.fetch });
}

export function getJudge(id: string, opts: { fetch?: FetchLike } = {}): Promise<Judge> {
  return request<Judge>(`/api/admin/judges/${encodeURIComponent(id)}`, {
    fetch: opts.fetch,
  });
}

export function createJudge(
  body: CreateJudgeBody,
  opts: { fetch?: FetchLike } = {},
): Promise<Judge> {
  return request<Judge>("/api/admin/judges", {
    method: "POST",
    body,
    fetch: opts.fetch,
  });
}

export function updateJudge(
  id: string,
  body: UpdateJudgeBody,
  opts: { fetch?: FetchLike } = {},
): Promise<Judge> {
  return request<Judge>(`/api/admin/judges/${encodeURIComponent(id)}`, {
    method: "PUT",
    body,
    fetch: opts.fetch,
  });
}

export function deleteJudge(id: string, opts: { fetch?: FetchLike } = {}): Promise<void> {
  return request<void>(`/api/admin/judges/${encodeURIComponent(id)}`, {
    method: "DELETE",
    fetch: opts.fetch,
  });
}

export function listTaskJudges(
  projectId: string,
  taskId: string,
  opts: { fetch?: FetchLike } = {},
): Promise<TaskJudge[]> {
  return request<TaskJudge[]>(
    `/api/projects/${encodeURIComponent(projectId)}/tasks/${encodeURIComponent(taskId)}/judges`,
    { fetch: opts.fetch },
  );
}

export function attachTaskJudge(
  projectId: string,
  taskId: string,
  body: AttachTaskJudgeBody,
  opts: { fetch?: FetchLike } = {},
): Promise<TaskJudge> {
  return request<TaskJudge>(
    `/api/projects/${encodeURIComponent(projectId)}/tasks/${encodeURIComponent(taskId)}/judges`,
    { method: "POST", body, fetch: opts.fetch },
  );
}

export function updateTaskJudge(
  projectId: string,
  taskId: string,
  judgeId: string,
  body: UpdateTaskJudgeBody,
  opts: { fetch?: FetchLike } = {},
): Promise<TaskJudge> {
  return request<TaskJudge>(
    `/api/projects/${encodeURIComponent(projectId)}/tasks/${encodeURIComponent(taskId)}/judges/${encodeURIComponent(judgeId)}`,
    { method: "PUT", body, fetch: opts.fetch },
  );
}

export function detachTaskJudge(
  projectId: string,
  taskId: string,
  judgeId: string,
  opts: { fetch?: FetchLike } = {},
): Promise<void> {
  return request<void>(
    `/api/projects/${encodeURIComponent(projectId)}/tasks/${encodeURIComponent(taskId)}/judges/${encodeURIComponent(judgeId)}`,
    { method: "DELETE", fetch: opts.fetch },
  );
}

export function runJudge(
  sessionCode: string,
  playerId: string,
  req: JudgeRunRequest,
  opts: { fetch?: FetchLike } = {},
): Promise<JudgeRunResult> {
  return request<JudgeRunResult>(
    `/api/admin/sessions/${encodeURIComponent(sessionCode)}/players/${encodeURIComponent(playerId)}/judge-runs`,
    { method: "POST", body: req, fetch: opts.fetch },
  );
}

/** Admin-only: full run details (model, provider, tokens, run log, error). */
export function getJudgeResultDetails(
  judgeResultId: string,
  opts: { fetch?: FetchLike } = {},
): Promise<import("$lib/types/arena").JudgeResultDetails> {
  return request<import("$lib/types/arena").JudgeResultDetails>(
    `/api/admin/judge-results/${encodeURIComponent(judgeResultId)}`,
    { fetch: opts.fetch },
  );
}

/** Per-player judge LLM token usage for a session (admin cost view). */
export interface JudgeUsageEntry {
  player_id?: string | null;
  display_name: string;
  runs: number;
  scored: number;
  failed: number;
  tokens_input: number;
  tokens_output: number;
  tokens_cache_read: number;
  tokens_cache_write: number;
}

export interface JudgeUsageResp {
  players: JudgeUsageEntry[];
  totals: JudgeUsageEntry;
}

/** Admin-only: judge token spend per player for one session. */
export function getSessionJudgeUsage(
  sessionCode: string,
  opts: { fetch?: FetchLike } = {},
): Promise<JudgeUsageResp> {
  return request<JudgeUsageResp>(
    `/api/admin/sessions/${encodeURIComponent(sessionCode)}/judge-usage`,
    { fetch: opts.fetch },
  );
}

/** Admin-only: the raw on-disk judge log file for one player+task —
 * a JSON object keyed by judge slug with every judge's full telemetry. */
export function getPlayerTaskJudgeLog(
  sessionCode: string,
  playerId: string,
  taskId: string,
  opts: { fetch?: FetchLike } = {},
): Promise<unknown> {
  return request<unknown>(
    `/api/admin/sessions/${encodeURIComponent(sessionCode)}/players/${encodeURIComponent(playerId)}/tasks/${encodeURIComponent(taskId)}/judge-log`,
    { fetch: opts.fetch },
  );
}

/** One entry of the public review bench shown on the landing page. */
export interface PublicJudge {
  slug: string;
  name: string;
  description: string;
}

/**
 * Public list of point-awarding judges — no auth required. Penalty judges
 * (anti-cheat and friends) are excluded server-side: they are fair-play
 * controls, not reviews.
 */
export async function listPublicJudges(opts: { fetch?: FetchLike } = {}): Promise<PublicJudge[]> {
  const data = await request<{ judges: PublicJudge[] }>("/api/public/judges", {
    fetch: opts.fetch,
  });
  return data.judges;
}
