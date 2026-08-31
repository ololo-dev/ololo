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
