import type { PageServerLoad } from "./$types";
import { error, redirect } from "@sveltejs/kit";
import { getSessionByCode, getSessionCampaign, getSessionReport, ApiError } from "$lib/api";

export const load: PageServerLoad = async ({ params, fetch }) => {
  let session;
  try {
    session = await getSessionByCode(params.code, { fetch });
  } catch (err) {
    if (err instanceof ApiError && err.status === 404) {
      throw error(404, "Session not found");
    }
    if (err instanceof ApiError && err.status === 429) {
      // The join-code endpoint rate-limits per IP; a busy dashboard must say
      // "slow down", not pretend the server broke.
      throw error(429, "Too many requests — wait a moment and refresh.");
    }
    throw error(500, "Failed to load session");
  }

  if (session.state === "lobby") {
    throw redirect(307, `/s/${params.code}/lobby`);
  }

  let report = null;
  if (session.state === "finished" || session.state === "cancelled") {
    try {
      report = await getSessionReport(session.id, { fetch });
    } catch {
      report = null;
    }
  }

  // Campaign context, when this session plays a part of one: null for every
  // ordinary session, and best-effort — the board must render without it.
  const campaign = await getSessionCampaign(params.code, { fetch }).catch(() => null);

  return { session, report, campaign };
};
