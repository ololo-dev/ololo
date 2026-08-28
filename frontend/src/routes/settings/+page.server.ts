import { getAdminUsers, getCostsSummary, getGameServers, listAdminSessions } from "$lib/api";
import type { PageServerLoad } from "./$types";

/**
 * The Overview dashboard. Auth/admin gating happens in the layout load;
 * every fetch here degrades to "no data" instead of failing the page, so a
 * half-deployed backend still renders the tiles it can.
 */
export const load: PageServerLoad = async ({ fetch }) => {
  const [users, recent, finished, running, lobby, costs, servers] = await Promise.all([
    getAdminUsers({ fetch }).catch(() => null),
    listAdminSessions({ per_page: 6 }, { fetch }).catch(() => null),
    listAdminSessions({ status: "finished", per_page: 1 }, { fetch }).catch(() => null),
    listAdminSessions({ status: "running", per_page: 1 }, { fetch }).catch(() => null),
    listAdminSessions({ status: "lobby", per_page: 1 }, { fetch }).catch(() => null),
    getCostsSummary(30, { fetch }).catch(() => null),
    getGameServers({ fetch }).catch(() => null),
  ]);

  return {
    users,
    recentSessions: recent?.sessions ?? [],
    sessionTotal: recent?.total ?? 0,
    finishedTotal: finished?.total ?? 0,
    runningTotal: running?.total ?? 0,
    lobbyTotal: lobby?.total ?? 0,
    costs,
    gameServers: servers,
  };
};
