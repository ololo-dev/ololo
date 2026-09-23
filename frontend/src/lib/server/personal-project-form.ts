// Form plumbing for the personal-project form: the request it posts, and
// the failure a page returns when the server refuses it (the entered
// values ride along, so a page without JavaScript re-renders them).

import { fail } from "@sveltejs/kit";
import type { ApiError, PersonalProjectRequest } from "$lib/api";

export interface PersonalFormValues {
  name: string;
  description: string;
  tasks: { title: string; description: string }[];
  judges: string[];
  session_duration_secs: number;
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
      const task = (t ?? {}) as { title?: unknown; description?: unknown };
      return {
        title: String(task.title ?? "").trim(),
        description: String(task.description ?? "").trim(),
      };
    })
    .filter((t) => t.title.length > 0);
  const rawJudges = parseJson<unknown[]>(data.get("judges_json"), []);
  const judges = (Array.isArray(rawJudges) ? rawJudges : []).map(String);
  const duration = Number(data.get("session_duration_secs"));
  const session_duration_secs = Number.isFinite(duration) && duration > 0 ? duration : 0;

  const request: PersonalProjectRequest = {
    description,
    tasks: tasks.map((t) => (t.description ? t : { title: t.title })),
    judges,
  };
  if (name) request.name = name;
  if (session_duration_secs) request.session_duration_secs = session_duration_secs;
  return {
    request,
    values: { name, description, tasks, judges, session_duration_secs },
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
