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

/** A draft navigation map for a description; nothing is stored. */
export async function suggestPersonalTasks(
  description: string,
  name: string | undefined,
  opts: { fetch?: FetchLike } = {},
): Promise<PersonalTask[]> {
  const data = await request<{ tasks: PersonalTask[] }>("/api/personal-projects/suggest-tasks", {
    method: "POST",
    body: { description, name },
    fetch: opts.fetch,
  });
  return data.tasks;
}
