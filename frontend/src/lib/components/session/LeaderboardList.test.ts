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
});
