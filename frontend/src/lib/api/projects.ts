// Projects, project tasks, AI helpers, and export/import.

import { request, type FetchLike } from "./core";
import type {
  Project,
  ProjectTask,
  CreateProjectRequest,
  PatchProjectRequest,
  TaskInput,
  TaskDraft,
  ExportEnvelope,
  TaskPreviewItem,
  ProjectPart,
} from "./types";

export async function listProjects(
  include_archived = false,
  opts: { fetch?: FetchLike } = {},
): Promise<Project[]> {
  const url = include_archived ? "/api/projects?include_archived=true" : "/api/projects";
  const data = await request<{ projects: Project[] }>(url, {
    fetch: opts.fetch,
  });
  return data.projects;
}

export function createProject(req: CreateProjectRequest, opts: { fetch?: FetchLike } = {}) {
  return request<Project>("/api/projects", {
    method: "POST",
    body: req,
    fetch: opts.fetch,
  });
}

export function getProject(id: string, opts: { fetch?: FetchLike } = {}) {
  return request<Project>(`/api/projects/${encodeURIComponent(id)}`, {
    fetch: opts.fetch,
  });
}

export function patchProject(
  id: string,
  req: PatchProjectRequest,
  opts: { fetch?: FetchLike } = {},
) {
  return request<Project>(`/api/projects/${encodeURIComponent(id)}`, {
    method: "PATCH",
    body: req,
    fetch: opts.fetch,
  });
}

export function deleteProject(id: string, opts: { fetch?: FetchLike } = {}) {
  return request<void>(`/api/projects/${encodeURIComponent(id)}`, {
    method: "DELETE",
    fetch: opts.fetch,
  });
}

/** Public task-arc preview; 404s when the project hides its ladder
 * (`show_tasks: false`) or is private. */
export async function getTaskPreview(
  projectId: string,
  opts: { fetch?: FetchLike } = {},
): Promise<TaskPreviewItem[]> {
  const data = await request<{ tasks: TaskPreviewItem[] }>(
    `/api/projects/${encodeURIComponent(projectId)}/tasks/preview`,
    { fetch: opts.fetch },
  );
  return data.tasks;
}

export function getProjectBySlug(slug: string, opts: { fetch?: FetchLike } = {}) {
  return request<Project>(`/api/projects/by-slug/${encodeURIComponent(slug)}`, {
    fetch: opts.fetch,
  });
}

export interface ProjectJudge {
  slug: string;
  name: string;
  description: string;
  /** Judge avatar (ImageKit or external); absent when none is set. */
  avatar_url?: string | null;
}

/** Distinct judges attached across the project's tasks. */
export async function listProjectJudges(
  id: string,
  opts: { fetch?: FetchLike } = {},
): Promise<ProjectJudge[]> {
  const data = await request<{ judges: ProjectJudge[] }>(
    `/api/projects/${encodeURIComponent(id)}/judges`,
    { fetch: opts.fetch },
  );
  return data.judges;
}

/** Parts of a campaign in play order, with the caller's progression on each.
 * An ordinary project answers with an empty list. */
export async function getProjectParts(
  id: string,
  opts: { fetch?: FetchLike } = {},
): Promise<ProjectPart[]> {
  const data = await request<{ parts: ProjectPart[] }>(
    `/api/projects/${encodeURIComponent(id)}/parts`,
    { fetch: opts.fetch },
  );
  return data.parts;
}

export interface TopPlayer {
  rank: number;
  user_id: string;
  username: string | null;
  display_name: string;
  avatar_url: string | null;
  /**
   * The player's best single-session score in this project — not a total
   * across sessions. May be negative when judges took the session below zero.
   */
  game_points: number;
  sessions_played: number;
  best_placement: number | null;
  /** Campaign boards only: parts of the campaign this player has cleared.
   *  `null` on an ordinary project, where the idea means nothing. */
  parts_completed?: number | null;
}

export interface TopPlayersBoards {
  /** Every season the project has been played in. */
  players: TopPlayer[];
  /** The same board restricted to the current season. */
  season_players: TopPlayer[];
  /** Start of the current season (RFC 3339). */
  season_start: string;
  /** Campaign boards only: how many parts there are to clear. */
  parts_total?: number | null;
}

/** Both boards empty — the shape loaders fall back to when the call fails. */
export const EMPTY_TOP_PLAYERS: TopPlayersBoards = {
  players: [],
  season_players: [],
  season_start: "",
  parts_total: null,
};

/** Best players for the project: all time and the current season. */
export function getProjectTopPlayers(
  id: string,
  opts: { fetch?: FetchLike } = {},
): Promise<TopPlayersBoards> {
  return request<TopPlayersBoards>(`/api/projects/${encodeURIComponent(id)}/top-players`, {
    fetch: opts.fetch,
  });
}

export function getPrivateProjectBySlug(
  userId: string,
  slug: string,
  opts: { fetch?: FetchLike } = {},
) {
  return request<Project>(
    `/api/projects/u/${encodeURIComponent(userId)}/by-slug/${encodeURIComponent(slug)}`,
    { fetch: opts.fetch },
  );
}

export function getProjectCategories(opts: { fetch?: FetchLike } = {}): Promise<string[]> {
  return request<{ categories: string[] }>("/api/projects/categories", {
    fetch: opts.fetch,
  }).then((data) => data.categories);
}

export async function listProjectTasks(
  project_id: string,
  opts: { fetch?: FetchLike } = {},
): Promise<ProjectTask[]> {
  const data = await request<{ tasks: ProjectTask[] }>(
    `/api/projects/${encodeURIComponent(project_id)}/tasks`,
    { fetch: opts.fetch },
  );
  return data.tasks;
}

export function createProjectTask(
  project_id: string,
  task: TaskInput,
  opts: { fetch?: FetchLike } = {},
) {
  return request<ProjectTask>(`/api/projects/${encodeURIComponent(project_id)}/tasks`, {
    method: "POST",
    body: task,
    fetch: opts.fetch,
  });
}

export function patchProjectTask(
  project_id: string,
  task_id: string,
  task: TaskInput,
  opts: { fetch?: FetchLike } = {},
) {
  return request<ProjectTask>(
    `/api/projects/${encodeURIComponent(project_id)}/tasks/${encodeURIComponent(task_id)}`,
    { method: "PATCH", body: task, fetch: opts.fetch },
  );
}

export function deleteProjectTask(
  project_id: string,
  task_id: string,
  opts: { fetch?: FetchLike } = {},
) {
  return request<void>(
    `/api/projects/${encodeURIComponent(project_id)}/tasks/${encodeURIComponent(task_id)}`,
    { method: "DELETE", fetch: opts.fetch },
  );
}

export function reorderTasks(
  projectId: string,
  taskIds: string[],
  opts: { fetch?: FetchLike } = {},
) {
  return request<void>(`/api/projects/${encodeURIComponent(projectId)}/tasks/reorder`, {
    method: "PATCH",
    body: { task_ids: taskIds },
    fetch: opts.fetch,
  });
}

export function beautifyDescription(
  projectId: string,
  description: string,
  opts: { fetch?: FetchLike } = {},
) {
  return request<{ description: string }>(
    `/api/projects/${encodeURIComponent(projectId)}/beautify-description`,
    { method: "POST", body: { description }, fetch: opts.fetch },
  );
}

export function generateTasks(
  projectId: string,
  description: string,
  opts: { fetch?: FetchLike } = {},
) {
  return request<{ tasks: TaskDraft[] }>(
    `/api/projects/${encodeURIComponent(projectId)}/generate-tasks`,
    { method: "POST", body: { description }, fetch: opts.fetch },
  );
}

export function beautifyTaskDescription(
  projectId: string,
  description: string,
  opts: { fetch?: FetchLike } = {},
) {
  return request<{ description: string }>(
    `/api/projects/${encodeURIComponent(projectId)}/tasks/ai/beautify`,
    { method: "POST", body: { description }, fetch: opts.fetch },
  );
}

export function generateTaskTests(
  projectId: string,
  title: string,
  description: string,
  opts: { fetch?: FetchLike } = {},
) {
  return request<{
    command_template: string;
    answer_template: string;
    fixtures: unknown[];
  }>(`/api/projects/${encodeURIComponent(projectId)}/tasks/ai/generate`, {
    method: "POST",
    body: { title, description },
    fetch: opts.fetch,
  });
}

export function beautifyTaskDescriptionById(
  projectId: string,
  taskId: string,
  description: string,
  opts: { fetch?: FetchLike } = {},
) {
  return request<{ description: string }>(
    `/api/projects/${encodeURIComponent(projectId)}/tasks/${encodeURIComponent(taskId)}/ai/beautify`,
    { method: "POST", body: { description }, fetch: opts.fetch },
  );
}

export function generateTaskTestsById(
  projectId: string,
  taskId: string,
  title: string,
  description: string,
  opts: { fetch?: FetchLike } = {},
) {
  return request<{
    command_template: string;
    answer_template: string;
    fixtures: unknown[];
  }>(
    `/api/projects/${encodeURIComponent(projectId)}/tasks/${encodeURIComponent(taskId)}/ai/generate`,
    { method: "POST", body: { title, description }, fetch: opts.fetch },
  );
}

export function exportProject(
  id: string,
  opts: { fetch?: FetchLike } = {},
): Promise<ExportEnvelope> {
  return request<ExportEnvelope>(`/api/admin/projects/${encodeURIComponent(id)}/export`, {
    fetch: opts.fetch,
  });
}

export interface ReseedResponse {
  project_id: string;
  name: string;
  tasks_updated: number;
  tasks_inserted: number;
  tasks_deleted: number;
}

/** Re-read a seeded project's on-disk definition and update it in place (admin only). */
export function reseedProject(
  id: string,
  opts: { fetch?: FetchLike } = {},
): Promise<ReseedResponse> {
  return request<ReseedResponse>(`/api/admin/projects/${encodeURIComponent(id)}/reseed`, {
    method: "POST",
    fetch: opts.fetch,
  });
}

export async function importProject(
  file: File,
  opts: { fetch?: FetchLike } = {},
): Promise<{ project_id: string; name: string }> {
  const text = await file.text();
  const parsed = JSON.parse(text);
  return request<{ project_id: string; name: string }>("/api/admin/projects/import", {
    method: "POST",
    body: parsed,
    fetch: opts.fetch,
  });
}
