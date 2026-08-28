import { redirect, error } from "@sveltejs/kit";
import type { PageServerLoad } from "./$types";
import { ApiError, getSessionByCode } from "$lib/api";

export const load: PageServerLoad = async ({ params, fetch, setHeaders }) => {
  setHeaders({ "cache-control": "no-store" });

  try {
    const session = await getSessionByCode(params.code, { fetch });
    if (session.state !== "lobby") {
      throw redirect(307, `/s/${session.join_code}`);
    }
    return { session };
  } catch (err) {
    if (err instanceof ApiError && err.status === 404) {
      throw error(404, "Session not found");
    }
    if (err instanceof ApiError && err.status === 429) {
      // The join-code endpoint rate-limits per IP; a busy lobby must say
      // "slow down", not pretend the server broke.
      throw error(429, "Too many requests — wait a moment and refresh.");
    }
    throw err;
  }
};
