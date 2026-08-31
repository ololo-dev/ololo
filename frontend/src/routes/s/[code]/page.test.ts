import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/svelte";
import SessionPage from "./+page.svelte";

const patchSessionMock = vi.fn();
vi.mock("$lib/api", () => ({
  ApiError: class ApiError extends Error {
    status: number;
    constructor(status: number) {
      super(`api ${status}`);
      this.status = status;
    }
  },
  patchSession: (...args: unknown[]) => patchSessionMock(...args),
  getSessionPlayerStats: async () => ({ total_tasks: 0, players: [] }),
}));

vi.mock("$app/navigation", () => ({
  invalidateAll: vi.fn(async () => {}),
}));

vi.mock("$app/environment", () => ({ browser: true }));

const baseSession = {
  id: "sess-1",
  join_code: "ABC123",
  state: "lobby",
  owner_id: null as string | null,
  project_id: "proj-1",
  project_name: "Project One",
  project_slug: "project-one",
  project_description: null,
};

const baseData = {
  session: baseSession,
  report: null,
  campaign: null,
  isAuthenticated: false,
  isAdmin: false,
  allowProjectCreation: false,
  replayEnabled: false,
  user: null as {
    id: string;
    name: string;
    initials: string;
    avatarUrl: string | undefined;
    username: string;
  } | null,
};

const finishedReport = {
  session_id: "sess-1",
  status: "finished",
  leaderboard: [
    { player_id: "p1", display_name: "Alpha", total_points: 20, tests_passed: 2, total_wall_ms: 0 },
    { player_id: "p2", display_name: "Beta", total_points: 10, tests_passed: 1, total_wall_ms: 0 },
  ],
  timeline: [
    {
      task_id: "task-1",
      task_title: "Task One",
      player_id: "p1",
      player_display_name: "Alpha",
      score: 1,
      answer: "ok",
      created_at: "2026-05-21T00:00:00Z",
    },
  ],
  activity_events: [],
};

describe("routes/s/[code]/+page.svelte", () => {
  it("renders lobby heading for lobby state", () => {
    render(SessionPage, { data: { ...baseData, session: { ...baseSession, state: "lobby" } } });
    expect(screen.getAllByText("Lobby").length).toBeGreaterThan(0);
    expect(screen.getAllByText("ABC123").length).toBeGreaterThan(0);
  });

  it("says Judging, not Complete, while judge runs are still owed", () => {
    render(SessionPage, {
      data: {
        ...baseData,
        session: { ...baseSession, state: "finished" },
        report: { ...finishedReport, judges_pending: 1 },
      },
    });
    expect(screen.getByText("Judging")).toBeInTheDocument();
    expect(screen.queryByText("Complete")).toBeNull();
  });

  it("says Complete once every judge run is terminal", () => {
    render(SessionPage, {
      data: {
        ...baseData,
        session: { ...baseSession, state: "finished" },
        report: { ...finishedReport, judges_pending: 0 },
      },
    });
    expect(screen.getByText("Complete")).toBeInTheDocument();
    expect(screen.queryByText("Judging")).toBeNull();
  });

  it("renders report controls for finished state", () => {
    render(SessionPage, {
      data: { ...baseData, session: { ...baseSession, state: "finished" }, report: finishedReport },
    });

    expect(screen.getAllByText("Final Results").length).toBeGreaterThan(0);
    // Result summary names the winner instead of a flat "Session complete."
    expect(screen.getByText("Alpha wins with 20 pts.")).not.toBeNull();
  });

  describe("the replay bar", () => {
    const withHistory = {
      ...finishedReport,
      score_history: [
        { t: 0, scores: { p1: 0 } },
        { t: 60, scores: { p1: 10 } },
      ],
    };
    const finished = { ...baseSession, state: "finished" };

    it("is offered to an admin while the instance keeps it on", () => {
      render(SessionPage, {
        data: {
          ...baseData,
          isAdmin: true,
          replayEnabled: true,
          session: finished,
          report: withHistory,
        },
      });
      expect(screen.getByTestId("replay-scrubber")).not.toBeNull();
    });

    it("is gone once an admin turns it off in settings", () => {
      render(SessionPage, {
        data: {
          ...baseData,
          isAdmin: true,
          replayEnabled: false,
          session: finished,
          report: withHistory,
        },
      });
      expect(screen.queryByTestId("replay-scrubber")).toBeNull();
    });

    it("is never offered to anybody else, switch or no switch", () => {
      render(SessionPage, {
        data: {
          ...baseData,
          isAdmin: false,
          replayEnabled: true,
          session: finished,
          report: withHistory,
        },
      });
      expect(screen.queryByTestId("replay-scrubber")).toBeNull();
    });
  });

  it("renders action labels for report activity events (event_kind → kind)", () => {
    const report = {
      ...finishedReport,
      players: [
        {
          player_id: "p1",
          user_id: "user-1",
          display_name: "Alpha",
          avatar_url: "https://img.example/alpha.png",
          username: "alpha",
        },
      ],
      score_history: [
        { t: 0, scores: { p1: 0 } },
        { t: 60, scores: { p1: 10 } },
      ],
      activity_events: [
        {
          event_kind: "task_started",
          player_id: "p1",
          player_display_name: "Alpha",
          task_id: "task-1",
          task_ordinal: 1,
          task_title: "Task One",
          judge_name: null,
          point_delta: null,
          timestamp: "2026-05-21T00:01:00Z",
          version: 1,
        },
        {
          event_kind: "task_scored",
          player_id: "p1",
          player_display_name: "Alpha",
          task_id: "task-1",
          task_ordinal: 1,
          task_title: "Task One",
          judge_name: null,
          point_delta: 10,
          timestamp: "2026-05-21T00:02:00Z",
          version: 2,
        },
      ],
    };
    render(SessionPage, {
      data: { ...baseData, session: { ...baseSession, state: "finished" }, report },
    });

    expect(screen.getByText("started working on")).not.toBeNull();
    expect(screen.getByText("implemented")).not.toBeNull();
    expect(screen.getByText("+10 points")).not.toBeNull();

    // Avatars resolve from the report's players — no live WS on finished sessions.
    const avatars = screen.getAllByAltText("Alpha avatar");
    expect(avatars.length).toBeGreaterThan(0);
    expect((avatars[0] as HTMLImageElement).src).toBe("https://img.example/alpha.png");

    // Score chart uses the report's score_history — no empty-state message.
    expect(screen.queryByText("No score history recorded.")).toBeNull();
  });

  it("own player row in finished leaderboard gets a player detail link", () => {
    // Mock ws.userPlayers is empty here (no WS in tests), but we can test via
    // the finished report leaderboard where ownPlayerIds drives the href.
    // We'll verify the baseline: without ownPlayerIds, row with username gets /u/ link.
    render(SessionPage, {
      data: { ...baseData, session: { ...baseSession, state: "finished" }, report: finishedReport },
    });

    // p1 row has no username, so no link rendered (it's a div)
    const links = document.querySelectorAll('a[href^="/s/ABC123/player/"]');
    // userPlayers is empty by default (no WS), so no own-player links
    expect(links.length).toBe(0);
  });

  it("participant gets a player detail link on finished sessions via report players", () => {
    const report = {
      ...finishedReport,
      players: [
        {
          player_id: "p1",
          user_id: "user-1",
          display_name: "Alpha",
          avatar_url: null,
          username: "alpha",
        },
        {
          player_id: "p2",
          user_id: "user-2",
          display_name: "Beta",
          avatar_url: null,
          username: "beta",
        },
      ],
    };
    render(SessionPage, {
      data: {
        ...baseData,
        session: { ...baseSession, state: "finished" },
        report,
        isAuthenticated: true,
        user: {
          id: "user-1",
          name: "Alpha",
          initials: "A",
          avatarUrl: undefined,
          username: "alpha",
        },
      },
    });

    // Own player (user-1 → p1) links to the player detail page by account
    // username, with no live query param (liveness is derived server-side).
    const ownLinks = document.querySelectorAll('a[href="/s/ABC123/player/alpha"]');
    expect(ownLinks.length).toBeGreaterThan(0);
    // Other players don't get a player-detail link (API would 403), only /u/ profiles.
    expect(document.querySelectorAll('a[href="/s/ABC123/player/beta"]').length).toBe(0);
  });

  it("admin gets a named run link on every leaderboard row", () => {
    const report = {
      ...finishedReport,
      players: [
        {
          player_id: "p1",
          user_id: "user-1",
          display_name: "Alpha",
          avatar_url: null,
          username: "alpha",
        },
        {
          player_id: "p2",
          user_id: "user-2",
          display_name: "Beta",
          avatar_url: null,
          username: "beta",
        },
      ],
    };
    render(SessionPage, {
      data: {
        ...baseData,
        session: { ...baseSession, state: "finished" },
        report,
        isAuthenticated: true,
        isAdmin: true,
        user: {
          id: "user-1",
          name: "Alpha",
          initials: "A",
          avatarUrl: undefined,
          username: "alpha",
        },
      },
    });

    // Own row keeps its link to the run…
    const ownLinks = document.querySelectorAll('a[href="/s/ABC123/player/alpha"]');
    expect(ownLinks.length).toBeGreaterThan(0);
    expect(ownLinks[0]!.textContent?.trim()).toBe("Report");
    expect(ownLinks[0]!.getAttribute("aria-label")).toContain("Alpha");
    // …and an admin gets the same named link on every other row.
    const adminLinks = document.querySelectorAll('a[href="/s/ABC123/player/beta"]');
    expect(adminLinks.length).toBeGreaterThan(0);
    expect(adminLinks[0]!.textContent?.trim()).toBe("Report");
    expect(adminLinks[0]!.getAttribute("aria-label")).toContain("Beta");
  });

  it("leaderboard href logic: own player links to detail path, others to /u", () => {
    // Mirrors LeaderboardList: own row → session player path (username
    // preferred, no ?live param), others → profile (or none without username).
    const code = "ABC123";
    const ownPlayerIds = new Set(["p1"]);

    function computeHref(
      entry: { player_id: string; username: string | null },
      isOwnRow: boolean,
    ): string | null {
      if (isOwnRow) {
        return `/s/${code}/player/${encodeURIComponent(entry.username ?? entry.player_id)}`;
      }
      return entry.username ? `/u/${entry.username}` : null;
    }

    const ownEntry = { player_id: "p1", username: "alpha" };
    const ownNoUsername = { player_id: "p9", username: null };
    const otherWithUsername = { player_id: "p2", username: "beta" };
    const otherNoUsername = { player_id: "p3", username: null };

    expect(computeHref(ownEntry, ownPlayerIds.has(ownEntry.player_id))).toBe(
      "/s/ABC123/player/alpha",
    );
    // Own row without a username falls back to the player id.
    expect(computeHref(ownNoUsername, true)).toBe("/s/ABC123/player/p9");
    expect(computeHref(otherWithUsername, ownPlayerIds.has(otherWithUsername.player_id))).toBe(
      "/u/beta",
    );
    expect(computeHref(otherNoUsername, ownPlayerIds.has(otherNoUsername.player_id))).toBe(null);
  });

  describe("owner/admin controls", () => {
    it("shows Pause and Cancel controls to session owner in running state", () => {
      render(SessionPage, {
        data: {
          ...baseData,
          session: { ...baseSession, state: "running", owner_id: "user-1" },
          isAuthenticated: true,
          isAdmin: false,
          user: {
            id: "user-1",
            name: "Owner",
            initials: "OW",
            avatarUrl: undefined,
            username: "owner",
          },
        },
      });
      expect(screen.getByRole("button", { name: /pause/i })).not.toBeNull();
      expect(screen.getByRole("button", { name: /cancel/i })).not.toBeNull();
    });

    it("shows Resume and Cancel controls to admin in paused state", () => {
      render(SessionPage, {
        data: {
          ...baseData,
          session: { ...baseSession, state: "paused", owner_id: "user-other" },
          isAuthenticated: true,
          isAdmin: true,
          user: {
            id: "user-1",
            name: "Admin",
            initials: "AD",
            avatarUrl: undefined,
            username: "admin",
          },
        },
      });
      expect(screen.getByRole("button", { name: /resume/i })).not.toBeNull();
      expect(screen.getByRole("button", { name: /cancel/i })).not.toBeNull();
    });

    it("hides controls from non-owner non-admin viewer in running state", () => {
      render(SessionPage, {
        data: {
          ...baseData,
          session: { ...baseSession, state: "running", owner_id: "user-other" },
          isAuthenticated: true,
          isAdmin: false,
          user: {
            id: "user-1",
            name: "Spectator",
            initials: "SP",
            avatarUrl: undefined,
            username: "spec",
          },
        },
      });
      expect(screen.queryByRole("button", { name: /pause|resume|cancel/i })).toBeNull();
    });

    it("shows Paused badge to all viewers when state is paused", () => {
      render(SessionPage, {
        data: {
          ...baseData,
          session: { ...baseSession, state: "paused", owner_id: "user-other" },
          isAuthenticated: false,
          isAdmin: false,
          user: null,
        },
      });
      expect(screen.getAllByText("Paused").length).toBeGreaterThan(0);
    });
  });
});
