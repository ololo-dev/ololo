import type { PageServerLoad } from "./$types";
import { error } from "@sveltejs/kit";
import { getPrivateProjectBySlug, listProjectTasks, ApiError } from "$lib/api";

export const load: PageServerLoad = async ({ params, fetch }) => {
  try {
    const project = await getPrivateProjectBySlug(params.userId, params.slug, {
      fetch,
    });
    const tasks = await listProjectTasks(project.id, { fetch });
    return { project, tasks };
  } catch (err) {
    if (err instanceof ApiError) {
      if (err.status === 404 || err.status === 403) throw error(404, "Project not found");
    }
    throw err;
  }
};
