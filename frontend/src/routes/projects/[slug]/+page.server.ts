import type { PageServerLoad, Actions } from "./$types";
import { fail, error, redirect } from "@sveltejs/kit";
import {
  getProjectBySlug,
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

export const load: PageServerLoad = async ({ params, fetch, url, locals }) => {
  const message = url.searchParams.get("message");
  try {
    const project = await getProjectBySlug(params.slug, { fetch });
    // Campaign context. A parent lists its own parts; a part loads its
    // campaign's list too, because that is where its own lock state lives.
    const campaignId =
      (project.part_count ?? 0) > 0 ? project.id : (project.parent_project_id ?? null);
    // A campaign hosts no sessions of its own — its parts do — so the page
    // neither asks for them nor shows the tab.
    const isCampaign = (project.part_count ?? 0) > 0;
    const [sessions, judges, topPlayers, taskPreview, parts] = await Promise.all([
      isCampaign ? Promise.resolve([]) : listProjectSessions(project.id, { fetch }).catch(() => []),
      listProjectJudges(project.id, { fetch }).catch(() => []),
      getProjectTopPlayers(project.id, { fetch }).catch(() => EMPTY_TOP_PLAYERS),
      // 404s when the project hides its ladder (show_tasks: false) for this
      // viewer — the page simply shows no task section then.
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
    }
    throw err;
  }
};

export const actions: Actions = {
  archive: async ({ params, fetch }) => {
    try {
      const project = await getProjectBySlug(params.slug, { fetch });
      await patchProject(project.id, { archived: true }, { fetch });
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
      const project = await getProjectBySlug(params.slug, { fetch });
      await patchProject(project.id, { archived: false }, { fetch });
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
      const project = await getProjectBySlug(params.slug, { fetch });
      await deleteProject(project.id, { fetch });
    } catch (err) {
      if (err instanceof ApiError) {
        return fail(err.status, { action: "delete", error: err.code ?? "error" });
      }
      throw err;
    }
    redirect(303, "/projects");
  },
};
