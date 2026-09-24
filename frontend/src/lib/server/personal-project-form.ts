// Form plumbing for the personal-project form: the request it posts, and
// the failure a page returns when the server refuses it (the entered
// values ride along, so a page without JavaScript re-renders them).

import { fail } from "@sveltejs/kit";
import type { ApiError, PersonalProjectRequest } from "$lib/api";

export interface PersonalFormValues {
  name: string;
  description: string;
  /** `judges`: the task's own panel; absent = the project's panel. */
  tasks: { title: string; description: string; judges?: string[] }[];
  judges: string[];
  session_duration_secs: number;
  public: boolean;
  /** Empty = no repository. */
  repo_url: string;
  repo_ref: string;
}

function slugs(raw: unknown): string[] {
  return (Array.isArray(raw) ? raw : []).map(String);
}

function parseJson<T>(raw: FormDataEntryValue | null, fallback: T): T {
  try {
    const value = JSON.parse(String(raw ?? ""));
    return value ?? fallback;
  } catch {
    return fallback;
  }
}

/** The request the form posts, and the values to re-render on failure. */
export function parsePersonalForm(data: FormData): {
  request: PersonalProjectRequest;
  values: PersonalFormValues;
} {
  const name = String(data.get("name") ?? "").trim();
  const description = String(data.get("description") ?? "").trim();
  const rawTasks = parseJson<unknown[]>(data.get("tasks_json"), []);
  const tasks = (Array.isArray(rawTasks) ? rawTasks : [])
    .map((t) => {
      const task = (t ?? {}) as { title?: unknown; description?: unknown; judges?: unknown };
      return {
        title: String(task.title ?? "").trim(),
        description: String(task.description ?? "").trim(),
        // An empty own panel is sent as is: the server refuses it by name
        // rather than the form quietly falling back to the default.
        ...(Array.isArray(task.judges) ? { judges: slugs(task.judges) } : {}),
      };
    })
    .filter((t) => t.title.length > 0);
  const judges = slugs(parseJson<unknown[]>(data.get("judges_json"), []));
  const duration = Number(data.get("session_duration_secs"));
  const session_duration_secs = Number.isFinite(duration) && duration > 0 ? duration : 0;
  // Public unless the form says, in so many words, private.
  const isPublic = String(data.get("public") ?? "true") !== "false";
  // Always sent, so a cleared field drops the repository on an edit.
  const repo_url = String(data.get("repo_url") ?? "").trim();
  const repo_ref = repo_url ? String(data.get("repo_ref") ?? "").trim() : "";

  const request: PersonalProjectRequest = {
    description,
    tasks: tasks.map(({ description, ...t }) => (description ? { ...t, description } : t)),
    judges,
    public: isPublic,
    repo_url,
    repo_ref,
  };
  if (name) request.name = name;
  if (session_duration_secs) request.session_duration_secs = session_duration_secs;
  return {
    request,
    values: {
      name,
      description,
      tasks,
      judges,
      session_duration_secs,
      public: isPublic,
      repo_url,
      repo_ref,
    },
  };
}

/** A refused request as the page's `form`: the code, the field and the
 *  server's detail when it named them, and the values. */
export function personalFailure(err: ApiError, values: PersonalFormValues) {
  const body = (err.body ?? {}) as { field?: unknown; detail?: unknown };
  const error =
    err.status === 403 && err.code?.startsWith("project creation")
      ? "creation_restricted"
      : (err.code ?? "error");
  return fail(err.status, {
    error,
    field: typeof body.field === "string" ? body.field : null,
    detail: typeof body.detail === "string" ? body.detail : null,
    values,
  });
}
