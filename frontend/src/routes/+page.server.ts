import type { PageServerLoad, Actions } from "./$types";
import { fail, redirect } from "@sveltejs/kit";
import {
  listProjects,
  createSession,
  getProjectCategories,
  listActiveSessions,
  getProjectCreationOpen,
  ApiError,
} from "$lib/api";

export const load: PageServerLoad = async ({ fetch, locals }) => {
  // Fetch projects for both authenticated and unauthenticated visitors.
  // Unauthenticated callers receive only public projects (backend returns 200
  // via Option<AccessClaims>); authenticated non-admins see own + public.
  // Catches remain for transient backend errors — the landing renders
  // without either section rather than 500ing.
  const [projects, activeSessions, categories, projectCreationOpen] = await Promise.all([
    listProjects(false, { fetch }).catch(() => []),
    listActiveSessions({ fetch }).catch(() => []),
    getProjectCategories({ fetch }).catch(() => []),
    // Whether signing up can lead to a project of one's own: what a
    // visitor's "Play your own work" block depends on. A signed-in
    // visitor's own rights come with the layout.
    locals.isAuthenticated
      ? Promise.resolve(false)
      : getProjectCreationOpen({ fetch }).catch(() => false),
  ]);
  return {
    // Personal projects are their owners' own work, not the catalog.
    projects: projects.filter((p) => p.kind !== "personal"),
    activeSessions,
    categories,
    projectCreationOpen,
  };
};

export const actions: Actions = {
  create: async ({ request, fetch }) => {
    const data = await request.formData();
    const name = String(data.get("name") ?? "").trim();
    const project_id = String(data.get("project_id") ?? "").trim();
    if (name.length < 1 || name.length > 120) {
      return fail(422, { error: "invalid_name", name, project_id });
    }
    if (!project_id) {
      return fail(422, { error: "project_required", name, project_id });
    }
    try {
      const session = await createSession(name, project_id, { fetch });
      throw redirect(303, `/s/${session.join_code}`);
    } catch (err) {
      if (err instanceof ApiError) {
        return fail(err.status, { error: err.code ?? "error", name, project_id });
      }
      throw err;
    }
  },
};
