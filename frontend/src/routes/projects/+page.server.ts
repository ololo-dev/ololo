import type { PageServerLoad } from "./$types";
import { listProjects, getProjectCategories, ApiError } from "$lib/api";

export const load: PageServerLoad = async ({ locals, fetch }) => {
  const [projects, categories] = await Promise.all([
    listProjects(true, { fetch }).catch((err) => {
      if (err instanceof ApiError && err.status === 401) return [];
      throw err;
    }),
    getProjectCategories({ fetch }).catch(() => []),
  ]);
  return { projects, categories, currentUserId: locals.userId };
};
