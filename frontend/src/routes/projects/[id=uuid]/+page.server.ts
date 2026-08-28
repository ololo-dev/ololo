import { redirect, error } from "@sveltejs/kit";
import { fail } from "@sveltejs/kit";
import {
  getProject,
  listProjectSessions,
  listProjectJudges,
  getProjectTopPlayers,
  getProjectParts,
  getTaskPreview,
  EMPTY_TOP_PLAYERS,
  patchProject,
  deleteProject,
  ApiError,
} from "$lib/api";
import type { PageServerLoad, Actions } from "./$types";

export const load: PageServerLoad = async ({ params, fetch, url, locals }) => {
  const message = url.searchParams.get("message");
  try {
    const project = await getProject(params.id, { fetch });
    // Redirect to the canonical slug URL when a slug is set.
    if (project.slug) {
      throw redirect(301, `/projects/${project.slug}`);
    }
    // No slug yet — render the project detail page directly.
    const campaignId =
      (project.part_count ?? 0) > 0 ? project.id : (project.parent_project_id ?? null);
    const [sessions, judges, topPlayers, taskPreview, parts] = await Promise.all([
      listProjectSessions(project.id, { fetch }).catch(() => []),
      listProjectJudges(project.id, { fetch }).catch(() => []),
      getProjectTopPlayers(project.id, { fetch }).catch(() => EMPTY_TOP_PLAYERS),
      getTaskPreview(project.id, { fetch }).catch(() => []),
      campaignId ? getProjectParts(campaignId, { fetch }).catch(() => []) : Promise.resolve([]),
    ]);
    return {
      project,
      sessions,
      judges,
      topPlayers,
      taskPreview,
      parts,
      message,
      currentUserId: locals.userId,
    };
  } catch (err) {
    if (err instanceof ApiError) {
      if (err.status === 404) throw error(404, "Project not found");
      if (err.status === 403) throw error(403, "Access denied");
      if (err.status === 401) throw error(401, "Login required");
    }
    throw err;
  }
};

export const actions: Actions = {
  archive: async ({ params, fetch }) => {
    try {
      await patchProject(params.id, { archived: true }, { fetch });
      return { action: "archive", success: true };
    } catch (err) {
      if (err instanceof ApiError) {
        return fail(err.status, { action: "archive", error: err.code ?? "error" });
      }
      throw err;
    }
  },

  unarchive: async ({ params, fetch }) => {
    try {
      await patchProject(params.id, { archived: false }, { fetch });
      return { action: "unarchive", success: true };
    } catch (err) {
      if (err instanceof ApiError) {
        return fail(err.status, { action: "unarchive", error: err.code ?? "error" });
      }
      throw err;
    }
  },

  delete: async ({ params, fetch }) => {
    try {
      await deleteProject(params.id, { fetch });
    } catch (err) {
      if (err instanceof ApiError) {
        return fail(err.status, { action: "delete", error: err.code ?? "error" });
      }
      throw err;
    }
    redirect(303, "/projects");
  },
};
