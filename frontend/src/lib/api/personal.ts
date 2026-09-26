// Personal projects: a user's own work, played as a normal session in their
// own repository (`/api/personal-projects`).

import { request, type FetchLike } from "./core";
import type {
  PersonalProjectDetail,
  PersonalProjectOptions,
  PersonalProjectRequest,
  PersonalTask,
  Project,
} from "./types";

export function getPersonalProjectOptions(opts: { fetch?: FetchLike } = {}) {
  return request<PersonalProjectOptions>("/api/personal-projects/options", {
    fetch: opts.fetch,
  });
}

export function createPersonalProject(
  req: PersonalProjectRequest,
  opts: { fetch?: FetchLike } = {},
) {
  return request<Project>("/api/personal-projects", {
    method: "POST",
    body: req,
    fetch: opts.fetch,
  });
}

export function getPersonalProject(id: string, opts: { fetch?: FetchLike } = {}) {
  return request<PersonalProjectDetail>(`/api/personal-projects/${encodeURIComponent(id)}`, {
    fetch: opts.fetch,
  });
}

/** Rebuild the tasks from a new request — until the first session. */
export function updatePersonalProject(
  id: string,
  req: PersonalProjectRequest,
  opts: { fetch?: FetchLike } = {},
) {
  return request<Project>(`/api/personal-projects/${encodeURIComponent(id)}`, {
    method: "PUT",
    body: req,
    fetch: opts.fetch,
  });
}

/** A project drafted from a description: a name for the work, when the
 *  model gave one, and its navigation map. */
export interface SuggestedProject {
  name: string | null;
  tasks: PersonalTask[];
}

/** Draft a project for a description; nothing is stored. */
export async function suggestPersonalProject(
  description: string,
  name: string | undefined,
  opts: { fetch?: FetchLike } = {},
): Promise<SuggestedProject> {
  const data = await request<{ name?: string | null; tasks: PersonalTask[] }>(
    "/api/personal-projects/suggest-tasks",
    { method: "POST", body: { description, name }, fetch: opts.fetch },
  );
  return { name: data.name ?? null, tasks: data.tasks };
}

/** A draft navigation map for a description; nothing is stored. */
export async function suggestPersonalTasks(
  description: string,
  name: string | undefined,
  opts: { fetch?: FetchLike } = {},
): Promise<PersonalTask[]> {
  return (await suggestPersonalProject(description, name, opts)).tasks;
}

/** Whether signed-in users may create their own projects on this instance
 *  (no auth needed): the landing offers a visitor its "describe your work"
 *  block only when signing up can lead to a project. */
export async function getProjectCreationOpen(opts: { fetch?: FetchLike } = {}): Promise<boolean> {
  const data = await request<{ allowed: boolean }>("/api/public/project-creation", {
    fetch: opts.fetch,
  });
  return data.allowed === true;
}
