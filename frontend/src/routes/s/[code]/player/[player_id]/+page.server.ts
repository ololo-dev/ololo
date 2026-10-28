import type { PageServerLoad } from "./$types";
import { error, redirect } from "@sveltejs/kit";
import { ApiError } from "$lib/api";
import type {
  PlayerHistoryResponse,
  PlayerSnapshotPayload,
  TaskStatsResponse,
} from "$lib/types/arena";

const ACCESS_COOKIE = "arena_access";

export const load: PageServerLoad = async ({ params, url, fetch, cookies }) => {
  const token = cookies.get(ACCESS_COOKIE);
  if (!token) {
    throw redirect(303, `/login?next=${encodeURIComponent(url.pathname + url.search)}`);
  }

  let snapshot: PlayerSnapshotPayload;
  try {
    snapshot = await fetch(
      `/api/sessions/${encodeURIComponent(params.code)}/players/${encodeURIComponent(params.player_id)}`,
    ).then(async (resp) => {
      if (resp.status === 403) throw new ApiError(403, null, null);
      if (resp.status === 404) throw new ApiError(404, null, null);
      if (!resp.ok) throw new ApiError(resp.status, null, null);
      return resp.json() as Promise<PlayerSnapshotPayload>;
    });
  } catch (err) {
    if (err instanceof ApiError) {
      if (err.status === 403) throw error(403, "Forbidden");
      if (err.status === 404) throw error(404, "Not found");
    }
    throw error(500, "Failed to load player data");
  }

  // The three side loads below are best-effort and independent, so they
  // run together: one after another they added their latencies up, and
  // with a workshop's worth of players refreshing, opening a player took
  // seconds.

  // Git history: a 403 is not a failure but a boundary — a signed-in
  // spectator may read the run, while diffs and files stay with the player
  // and admins; the page hides the inspection tabs instead of showing them
  // broken.
  const loadHistory = async (): Promise<{
    history: PlayerHistoryResponse | null;
    inspectRestricted: boolean;
  }> => {
    try {
      const histResp = await fetch(
        `/api/sessions/${encodeURIComponent(params.code)}/players/${encodeURIComponent(params.player_id)}/history`,
      );
      if (histResp.ok) {
        return {
          history: (await histResp.json()) as PlayerHistoryResponse,
          inspectRestricted: false,
        };
      }
      return { history: null, inspectRestricted: histResp.status === 403 };
    } catch {
      return { history: null, inspectRestricted: false };
    }
  };

  // Task agent statistics.
  const loadTaskStats = async (): Promise<TaskStatsResponse | null> => {
    try {
      const statsResp = await fetch(
        `/api/sessions/${encodeURIComponent(params.code)}/players/${encodeURIComponent(params.player_id)}/task-stats`,
      );
      return statsResp.ok ? ((await statsResp.json()) as TaskStatsResponse) : null;
    } catch {
      return null;
    }
  };

  // Judge avatars (slug → url) from the public project judges list; the
  // session-by-code lookup carries the project id.
  const loadJudges = async (): Promise<{
    judgeAvatars: Record<string, string>;
    sessionId: string | null;
  }> => {
    const judgeAvatars: Record<string, string> = {};
    let sessionId: string | null = null;
    try {
      const sess = (await fetch(`/api/sessions/by-code/${encodeURIComponent(params.code)}`).then(
        (r) => (r.ok ? r.json() : null),
      )) as { id?: string; project_id?: string | null } | null;
      sessionId = sess?.id ?? null;
      if (sess?.project_id) {
        const resp = await fetch(`/api/projects/${encodeURIComponent(sess.project_id)}/judges`);
        if (resp.ok) {
          const body = (await resp.json()) as {
            judges?: { slug: string; avatar_url?: string | null }[];
          };
          for (const j of body.judges ?? []) {
            if (j.avatar_url) judgeAvatars[j.slug] = j.avatar_url;
          }
        }
      }
    } catch {
      // non-fatal
    }
    return { judgeAvatars, sessionId };
  };

  const [{ history, inspectRestricted }, taskStats, { judgeAvatars, sessionId }] =
    await Promise.all([loadHistory(), loadTaskStats(), loadJudges()]);

  // A session is "live" (WS updates + Live badge) unless it has ended.
  // Derived from the snapshot status rather than a URL query param.
  const live = snapshot.session_status !== "finished" && snapshot.session_status !== "cancelled";

  // A finished (not cancelled) session whose judges have not all settled still
  // needs the live socket: judge verdicts publish AFTER finish, so without it
  // the page freezes on this snapshot and judges never appear in realtime.
  // Mirrors the server player-WS gate, which stays open through this window.
  // Capped by staleness: a session whose judge runs were lost for good (no
  // sign of life for 30+ minutes) must not hold a live socket forever.
  const JUDGES_STALE_AFTER_MS = 30 * 60 * 1000;
  const lastJudgeSignalMs = Math.max(
    snapshot.session_ends_at ? Date.parse(snapshot.session_ends_at) : 0,
    ...(snapshot.judge_statuses ?? []).map((s) => (s.updated_at ? Date.parse(s.updated_at) : 0)),
  );
  const judgesStale =
    lastJudgeSignalMs > 0 && Date.now() - lastJudgeSignalMs > JUDGES_STALE_AFTER_MS;
  const judgesSettling =
    snapshot.session_status === "finished" &&
    !judgesStale &&
    (snapshot.judge_statuses ?? []).some((s) => s.status !== "scored" && s.status !== "failed");

  return {
    live,
    judgesSettling,
    snapshot,
    history,
    inspectRestricted,
    taskStats,
    judgeAvatars,
    sessionId,
    token: live || judgesSettling ? token : null,
    playerName: snapshot.display_name,
  };
};
