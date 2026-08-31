import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/svelte";
import LeaderboardList from "./LeaderboardList.svelte";

function makeEntry(overrides: Record<string, unknown> = {}) {
  return {
    player_id: "p1",
    display_name: "Alpha",
    total_points: 10,
    avatar_url: null,
    ...overrides,
  };
}

describe("LeaderboardList", () => {
  it("renders an In progress badge when completion_status is in_progress", () => {
    render(LeaderboardList, {
      entries: [makeEntry({ completion_status: "in_progress" })],
      joinCode: "ABC123",
      userPlayers: [],
      emptyMessage: "empty",
    });
    expect(screen.getByText("In progress")).not.toBeNull();
  });

  it("renders an Awaiting judges badge when completion_status is awaiting_judges", () => {
    render(LeaderboardList, {
      entries: [makeEntry({ completion_status: "awaiting_judges" })],
      joinCode: "ABC123",
      userPlayers: [],
      emptyMessage: "empty",
    });
    expect(screen.getByText("Awaiting judges")).not.toBeNull();
  });

  it("renders a Completed badge when completion_status is completed", () => {
    render(LeaderboardList, {
      entries: [makeEntry({ completion_status: "completed" })],
      joinCode: "ABC123",
      userPlayers: [],
      emptyMessage: "empty",
    });
    expect(screen.getByText("Completed")).not.toBeNull();
  });

  it("renders no completion badge when completion_status is absent", () => {
    render(LeaderboardList, {
      entries: [makeEntry()],
      joinCode: "ABC123",
      userPlayers: [],
      emptyMessage: "empty",
    });
    expect(screen.getByText("Alpha")).not.toBeNull();
    expect(screen.queryByText("In progress")).toBeNull();
    expect(screen.queryByText("Awaiting judges")).toBeNull();
    expect(screen.queryByText("Completed")).toBeNull();
  });
  /**
   * A signed-in spectator on a public project may open any player's run —
   * the pill renders on the same rule the snapshot endpoint enforces, so
   * the link exists exactly where the page behind it will answer.
   */
  it("offers the run link to spectators when canViewRuns is set", () => {
    render(LeaderboardList, {
      entries: [makeEntry({ display_name: "Rival" })],
      joinCode: "ABC123",
      userPlayers: [],
      emptyMessage: "empty",
      canViewRuns: true,
    });
    const link = screen.getByRole("link", { name: /Open Rival's run/ });
    expect(link.getAttribute("href")).toBe("/s/ABC123/player/p1");
  });

  /**
   * The card is 300px wide. With the status pill beside the name and a run
   * link at the end of the row, "Andrey" rendered as "A…" on ololo.dev — the
   * name is the one thing a leaderboard row exists to say, so it gets the
   * line to itself and the status goes down with the agent.
   */
  it("keeps the name on its own line, with the status beside the agent", () => {
    render(LeaderboardList, {
      entries: [
        makeEntry({
          display_name: "Andrey",
          agent_display_name: "opencode",
          completion_status: "in_progress",
        }),
      ],
      joinCode: "ABC123",
      userPlayers: [],
      emptyMessage: "empty",
    });
    const name = screen.getByText("Andrey");
    expect(name.textContent?.trim()).toBe("Andrey");
    expect(name.querySelector("span")).toBeNull();
    const badgeLine = screen.getByText("In progress").closest("p");
    expect(badgeLine).not.toBeNull();
    expect(badgeLine!.textContent).toContain("opencode");
    expect(badgeLine!.textContent).not.toContain("Andrey");
  });

  /**
   * The player run page — report, chat with the agent, task details, judge
   * verdicts — is reachable from nowhere else in the app. It used to hang off
   * a 14px icon at half opacity whose only explanation was a `title`.
   */
  it("names the link to a player's run instead of hiding it behind an icon", () => {
    render(LeaderboardList, {
      entries: [makeEntry({ username: "alpha" })],
      joinCode: "ABC123",
      userPlayers: [
        {
          player_id: "p1",
          user_id: "u1",
          display_name: "Alpha",
          fingerprint: null,
          joined_at: "2026-08-27T14:35:00Z",
          reconnected_at: null,
          revoked_at: null,
        },
      ],
      emptyMessage: "empty",
      detailLabel: "Report",
    });
    const link = screen.getByRole("link", { name: /Open Alpha's run/ });
    expect(link.getAttribute("href")).toBe("/s/ABC123/player/alpha");
    expect(link.textContent?.trim()).toBe("Report");
  });

  it("offers no run link for another player when the viewer is not an admin", () => {
    render(LeaderboardList, {
      entries: [makeEntry({ username: "alpha" })],
      joinCode: "ABC123",
      userPlayers: [],
      emptyMessage: "empty",
    });
    expect(screen.queryByRole("link", { name: /Open Alpha's run/ })).toBeNull();
  });
});
