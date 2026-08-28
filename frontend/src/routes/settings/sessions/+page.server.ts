import { listAdminSessions, listProjects } from "$lib/api";
import type { PageServerLoad } from "./$types";

/** Kept in step with `SessionStatus` in arena-core. */
const STATUSES = ["lobby", "running", "paused", "finished", "cancelled"];

export const load: PageServerLoad = async ({ url, fetch }) => {
  const page = Number(url.searchParams.get("page") ?? "1") || 1;
  // Filters live in the URL so a filtered registry can be linked to and
  // survives the reload that follows a cancel or a delete.
  const rawStatus = url.searchParams.get("status") ?? "";
  // The server rejects an unknown status rather than ignoring it; a stale or
  // hand-edited URL should show an unfiltered list, not a 422 error page.
  const status = STATUSES.includes(rawStatus) ? rawStatus : "";
  const projectId = url.searchParams.get("project_id") ?? "";
  const q = url.searchParams.get("q") ?? "";

  const [sessions, projects] = await Promise.all([
    listAdminSessions({ page, per_page: 25, status, project_id: projectId, q }, { fetch }),
    // Only for the filter dropdown, so an empty list costs the filter rather
    // than the page.
    listProjects(true, { fetch }).catch(() => []),
  ]);

  return { sessions, projects, filters: { status, projectId, q } };
};
