import { redirect, error } from "@sveltejs/kit";
import {
  getAdminUsers,
  listProjects,
  getCategories,
  listJudges,
  listAdminSessions,
} from "$lib/api";
import type { LayoutServerLoad } from "./$types";

export const load: LayoutServerLoad = async ({ locals, parent, fetch }) => {
  if (!locals.isAuthenticated) {
    throw redirect(303, "/login?next=/settings");
  }
  const { isAdmin } = await parent();
  if (!isAdmin) {
    throw error(403, "You must be an administrator to view this page.");
  }

  // Counts drive the tab badges. Fetched in parallel; a failing count
  // degrades to "no badge" rather than breaking the whole settings shell.
  const [users, projects, categories, judges, sessions] = await Promise.all([
    getAdminUsers({ fetch }).catch(() => []),
    listProjects(true, { fetch }).catch(() => []),
    getCategories({ fetch }).catch(() => []),
    listJudges({ fetch }).catch(() => []),
    // Only the total is wanted, so ask for the smallest page the server will
    // give rather than pulling every session in just to call `.length`.
    listAdminSessions({ per_page: 1 }, { fetch }).catch(() => ({ total: 0 })),
  ]);
  return {
    userCount: users.length,
    projectCount: projects.length,
    categoryCount: categories.length,
    judgeCount: judges.length,
    sessionCount: sessions.total,
  };
};
