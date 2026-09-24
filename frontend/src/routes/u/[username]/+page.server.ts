import { error } from "@sveltejs/kit";
import type { PageServerLoad } from "./$types";
import type { Project, PublicUserProfile, PublicSessionsResponse } from "$lib/api";

/** Sessions per page. The list is a history, not a feed: 118 of them arrived
 *  20 at a time with no way to reach the rest, and the heading counted all
 *  118 as if they were on the page. */
const PER_PAGE = 20;

export const load: PageServerLoad = async ({ params, fetch, url }) => {
  const username = params.username;
  const requested = Number(url.searchParams.get("page") ?? "1");
  const pageNo = Number.isFinite(requested) && requested >= 1 ? Math.floor(requested) : 1;

  const profileResp = await fetch(`/api/users/by-username/${encodeURIComponent(username)}`);

  if (!profileResp.ok) {
    if (profileResp.status === 404) {
      throw error(404, "User not found");
    }
    throw error(500, "Failed to load profile");
  }

  const profile: PublicUserProfile = await profileResp.json();

  const sessionsResp = await fetch(
    `/api/users/by-username/${encodeURIComponent(username)}/sessions?page=${pageNo}&per_page=${PER_PAGE}`,
  );

  const sessions: PublicSessionsResponse = sessionsResp.ok
    ? await sessionsResp.json()
    : { sessions: [], total: 0, page: pageNo, per_page: PER_PAGE };

  // The user's own projects — the public ones, and their private ones on
  // their own profile. A failed fetch costs the shelf, not the profile.
  const projectsResp = await fetch(
    `/api/users/by-username/${encodeURIComponent(username)}/projects`,
  );
  const projects: Project[] = projectsResp.ok
    ? ((await projectsResp.json()) as { projects: Project[] }).projects
    : [];

  return { profile, sessions, projects };
};
