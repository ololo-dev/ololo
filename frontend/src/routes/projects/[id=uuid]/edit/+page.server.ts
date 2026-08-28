import type { PageServerLoad, Actions } from "./$types";
import { fail, redirect, error } from "@sveltejs/kit";
import {
  getProject,
  listProjectTasks,
  getProjectCategories,
  createProjectTask,
  deleteProjectTask,
  patchProject,
  patchProjectTask,
  reorderTasks,
  beautifyDescription,
  generateTasks,
  beautifyTaskDescription,
  generateTaskTests,
  beautifyTaskDescriptionById,
  generateTaskTestsById,
  listJudges,
  listTaskJudges,
  ApiError,
  type TaskInput,
  type TaskDraft,
} from "$lib/api";
import {
  buildPointsFromForm,
  buildIntervalsFromForm,
  buildIdleTimeoutFromForm,
  buildMemorySchemaFromForm,
  buildSessionDurationFromForm,
  buildTestTemplate,
  parseFixtures,
  defaultBackoff,
} from "$lib/server/project-form";

export const load: PageServerLoad = async ({ params, fetch, locals, parent }) => {
  if (!locals.isAuthenticated) {
    throw redirect(303, `/login?next=/projects/${params.id}/edit`);
  }
  try {
    const [{ isAdmin }, project, tasks, categories] = await Promise.all([
      parent(),
      getProject(params.id, { fetch }),
      listProjectTasks(params.id, { fetch }),
      getProjectCategories({ fetch }).catch(() => [] as string[]),
    ]);
    if (locals.userId !== project.owner_user_id && !isAdmin) {
      throw error(403, "Access denied");
    }
    if (project.archived_at) {
      throw redirect(303, `/projects/${params.id}?message=archived`);
    }
    let judges: Awaited<ReturnType<typeof listJudges>> = [];
    const taskJudgesMap: Record<string, Awaited<ReturnType<typeof listTaskJudges>>> = {};
    if (isAdmin) {
      judges = await listJudges({ fetch }).catch(() => [] as never);
      await Promise.all(
        tasks.map(async (t) => {
          try {
            taskJudgesMap[t.id] = await listTaskJudges(params.id, t.id, { fetch });
          } catch {
            taskJudgesMap[t.id] = [];
          }
        }),
      );
    }
    return { project, tasks, categories, judges, taskJudgesMap };
  } catch (err) {
    if (err instanceof ApiError) {
      if (err.status === 404) throw error(404, "Project not found");
      if (err.status === 403) throw error(403, "Access denied");
    }
    throw err;
  }
};

export const actions: Actions = {
  // --- Project Actions ---
  editProject: async ({ params, request, fetch }) => {
    const data = await request.formData();
    const name = String(data.get("name") ?? "").trim();
    const description = String(data.get("description") ?? "").trim();
    // Only admins see the slug field; if absent, send undefined (no change).
    const slug_raw = data.get("slug");
    const slug = slug_raw !== null ? String(slug_raw).trim() : undefined;
    const isPublic = data.get("public") === "true";
    const category = String(data.get("category") ?? "").trim() || undefined;
    const tags_json = String(data.get("tags_json") ?? "[]");
    let tags: string[] | undefined;
    try {
      tags = JSON.parse(tags_json) as string[];
    } catch {
      tags = [];
    }
    const cover_image_url_raw = String(data.get("cover_image_url") ?? "").trim();
    const clear_cover_image = data.get("clear_cover_image") === "true";
    const cover_image_url = cover_image_url_raw || undefined;

    // Points defaults (admin only). Empty inputs → null (preserve on PATCH).
    const points = buildPointsFromForm(data);

    // Intervals defaults (admin only). Empty inputs → null (preserve on PATCH).
    const intervals = buildIntervalsFromForm(data);

    // Session duration default (admin only). Empty input → omit (preserve on PATCH).
    const session_duration_secs = buildSessionDurationFromForm(data);
    const idle_timeout_secs = buildIdleTimeoutFromForm(data);

    // Session-memory schema. `null` clears it; `undefined` omits the field.
    const memory_schema = buildMemorySchemaFromForm(data);

    if (name.length < 1 || name.length > 200) {
      return fail(422, {
        action: "editProject",
        error: "invalid_name",
        name,
        description,
        slug,
      });
    }

    try {
      const project = await patchProject(
        params.id,
        {
          name,
          description,
          slug,
          public: isPublic,
          category,
          tags,
          cover_image_url,
          clear_cover_image: clear_cover_image || undefined,
          points,
          intervals,
          session_duration_secs,
          idle_timeout_secs,
          memory_schema,
        },
        { fetch },
      );
      throw redirect(303, `/projects/${project.id}`);
    } catch (err) {
      if (err instanceof ApiError) {
        if (err.status === 409 && err.code === "project_archived") {
          throw redirect(303, `/projects/${params.id}?message=archived`);
        }
        return fail(err.status, {
          action: "editProject",
          error: err.code ?? "error",
          name,
          description,
          slug,
        });
      }
      throw err;
    }
  },

  // --- Task Actions ---
  addTask: async ({ params, request, fetch }) => {
    const data = await request.formData();
    const title = String(data.get("title") ?? "").trim();
    if (title.length < 1 || title.length > 200) {
      return fail(422, { action: "addTask", error: "invalid_title", title });
    }
    let tags: string[] = [];
    try {
      tags = JSON.parse(String(data.get("tags_json") ?? data.get("tags") ?? "[]")) as string[];
    } catch {
      tags = [];
    }
    const answer_template = String(data.get("answer_template") ?? "").trim();
    const command_template = String(data.get("command_template") ?? "").trim();
    const fixtures = parseFixtures(data.get("fixtures"));
    const task: TaskInput = {
      title,
      description: String(data.get("description") ?? "").trim(),
      tags,
      test_template: buildTestTemplate(command_template, answer_template, fixtures),
      points: buildPointsFromForm(data),
      intervals: buildIntervalsFromForm(data),
    };
    try {
      await createProjectTask(params.id, task, { fetch });
      return { action: "addTask", success: true };
    } catch (err) {
      if (err instanceof ApiError) {
        return fail(err.status, {
          action: "addTask",
          error: err.code ?? "error",
          title,
        });
      }
      throw err;
    }
  },

  deleteTask: async ({ params, request, fetch }) => {
    const data = await request.formData();
    const task_id = String(data.get("task_id") ?? "");
    try {
      await deleteProjectTask(params.id, task_id, { fetch });
      return { action: "deleteTask", success: true };
    } catch (err) {
      if (err instanceof ApiError) {
        return fail(err.status, {
          action: "deleteTask",
          error: err.code ?? "error",
        });
      }
      throw err;
    }
  },

  editTask: async ({ params, request, fetch }) => {
    const data = await request.formData();
    const task_id = String(data.get("task_id") ?? "");
    const title = String(data.get("title") ?? "").trim();
    if (title.length < 1 || title.length > 200) {
      return fail(422, { action: "editTask", error: "invalid_title", title });
    }
    const description = String(data.get("description") ?? "").trim();
    const command_template = String(data.get("command_template") ?? "").trim();
    const answer_template = String(data.get("answer_template") ?? "").trim();
    const fixtures = parseFixtures(data.get("fixtures"));
    const tags_raw = data.get("tags_json") ?? data.get("tags");
    let tags: string[] | undefined;
    if (tags_raw !== null) {
      try {
        tags = JSON.parse(String(tags_raw)) as string[];
      } catch {
        tags = [];
      }
    }
    const task: TaskInput = {
      title,
      description,
      ...(tags !== undefined ? { tags } : {}),
      test_template: buildTestTemplate(command_template, answer_template, fixtures),
      points: buildPointsFromForm(data),
      intervals: buildIntervalsFromForm(data),
    };
    try {
      await patchProjectTask(params.id, task_id, task, { fetch });
      return { action: "editTask", success: true };
    } catch (err) {
      if (err instanceof ApiError) {
        return fail(err.status, {
          action: "editTask",
          error: err.code ?? "error",
        });
      }
      throw err;
    }
  },

  reorderTasks: async ({ params, request, fetch }) => {
    const data = await request.formData();
    const taskIds = String(data.get("task_ids") ?? "")
      .split(",")
      .filter(Boolean);
    try {
      await reorderTasks(params.id, taskIds, { fetch });
      return { action: "reorderTasks", success: true };
    } catch (err) {
      if (err instanceof ApiError) {
        return fail(err.status, {
          action: "reorderTasks",
          error: err.code ?? "error",
        });
      }
      throw err;
    }
  },

  // --- AI Actions ---
  beautifyDescription: async ({ params, request, fetch }) => {
    const data = await request.formData();
    const description = String(data.get("description") ?? "").trim();
    if (!description) {
      return fail(422, {
        action: "beautifyDescription",
        error: "description_empty",
      });
    }
    try {
      const result = await beautifyDescription(params.id, description, {
        fetch,
      });
      return { action: "beautifyDescription", description: result.description };
    } catch (err) {
      if (err instanceof ApiError) {
        return fail(err.status, {
          action: "beautifyDescription",
          error: err.code ?? "error",
        });
      }
      throw err;
    }
  },

  generateTasks: async ({ params, request, fetch }) => {
    const data = await request.formData();
    const description = String(data.get("description") ?? "").trim();
    if (!description) {
      return fail(422, { action: "generateTasks", error: "description_empty" });
    }
    try {
      const result = await generateTasks(params.id, description, { fetch });
      return { action: "generateTasks", tasks: result.tasks };
    } catch (err) {
      if (err instanceof ApiError) {
        return fail(err.status, {
          action: "generateTasks",
          error: err.code ?? "error",
        });
      }
      throw err;
    }
  },

  beautifyTaskDescription: async ({ params, request, fetch }) => {
    const data = await request.formData();
    const description = String(data.get("description") ?? "").trim();
    if (!description) {
      return fail(422, {
        action: "beautifyTaskDescription",
        error: "description_empty",
      });
    }
    try {
      const result = await beautifyTaskDescription(params.id, description, {
        fetch,
      });
      return {
        action: "beautifyTaskDescription",
        description: result.description,
      };
    } catch (err) {
      if (err instanceof ApiError) {
        return fail(err.status, {
          action: "beautifyTaskDescription",
          error: err.code ?? "error",
        });
      }
      throw err;
    }
  },

  generateTaskTests: async ({ params, request, fetch }) => {
    const data = await request.formData();
    const title = String(data.get("title") ?? "").trim();
    const description = String(data.get("description") ?? "").trim();
    if (!title) {
      return fail(422, { action: "generateTaskTests", error: "title_empty" });
    }
    if (!description) {
      return fail(422, {
        action: "generateTaskTests",
        error: "description_empty",
      });
    }
    try {
      const result = await generateTaskTests(params.id, title, description, {
        fetch,
      });
      return {
        action: "generateTaskTests",
        command_template: result.command_template,
        answer_template: result.answer_template,
        fixtures: result.fixtures,
      };
    } catch (err) {
      if (err instanceof ApiError) {
        return fail(err.status, {
          action: "generateTaskTests",
          error: err.code ?? "error",
        });
      }
      throw err;
    }
  },

  beautifyTaskDescriptionById: async ({ params, request, fetch }) => {
    const data = await request.formData();
    const task_id = String(data.get("task_id") ?? "").trim();
    const description = String(data.get("description") ?? "").trim();
    if (!task_id) {
      return fail(422, {
        action: "beautifyTaskDescriptionById",
        error: "task_id_empty",
      });
    }
    if (!description) {
      return fail(422, {
        action: "beautifyTaskDescriptionById",
        error: "description_empty",
      });
    }
    try {
      const result = await beautifyTaskDescriptionById(params.id, task_id, description, { fetch });
      return {
        action: "beautifyTaskDescriptionById",
        description: result.description,
      };
    } catch (err) {
      if (err instanceof ApiError) {
        return fail(err.status, {
          action: "beautifyTaskDescriptionById",
          error: err.code ?? "error",
        });
      }
      throw err;
    }
  },

  generateTaskTestsById: async ({ params, request, fetch }) => {
    const data = await request.formData();
    const task_id = String(data.get("task_id") ?? "").trim();
    const title = String(data.get("title") ?? "").trim();
    const description = String(data.get("description") ?? "").trim();
    if (!task_id) {
      return fail(422, {
        action: "generateTaskTestsById",
        error: "task_id_empty",
      });
    }
    if (!title) {
      return fail(422, {
        action: "generateTaskTestsById",
        error: "title_empty",
      });
    }
    if (!description) {
      return fail(422, {
        action: "generateTaskTestsById",
        error: "description_empty",
      });
    }
    try {
      const result = await generateTaskTestsById(params.id, task_id, title, description, { fetch });
      return {
        action: "generateTaskTestsById",
        command_template: result.command_template,
        answer_template: result.answer_template,
        fixtures: result.fixtures,
      };
    } catch (err) {
      if (err instanceof ApiError) {
        return fail(err.status, {
          action: "generateTaskTestsById",
          error: err.code ?? "error",
        });
      }
      throw err;
    }
  },

  importTasks: async ({ params, request, fetch }) => {
    const data = await request.formData();
    const tasks_json = String(data.get("tasks_json") ?? "[]");
    let tasks: TaskDraft[];
    try {
      tasks = JSON.parse(tasks_json) as TaskDraft[];
    } catch {
      return fail(422, { action: "importTasks", error: "invalid_json" });
    }

    let imported = 0;
    let failed = 0;
    const total = tasks.length;

    for (const task of tasks) {
      try {
        const taskInput: TaskInput = {
          title: task.title.trim() || "Untitled Task",
          description: task.description,
          ...(task.tags !== undefined ? { tags: task.tags } : {}),
          test_template: {
            kind: "shell",
            command_template: task.command_template || "true",
            placeholders: [],
            backoff: defaultBackoff(),
          },
        };
        await createProjectTask(params.id, taskInput, { fetch });
        imported++;
      } catch (err) {
        console.error("[importTasks] createProjectTask failed:", err);
        failed++;
      }
    }

    return { action: "importTasks", imported, failed, total };
  },
};
