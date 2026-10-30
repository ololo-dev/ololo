import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/svelte";
import ScreenLeaderboard from "./ScreenLeaderboard.svelte";

const rows = [
  {
    player_id: "p1",
    display_name: "Alpha",
    agent_display_name: "claude",
    total_points: 648,
    health: {
      score: 78.4,
      grade: "B",
      level: "green" as const,
      trend: "up" as const,
      verified: true,
      points: 3,
    },
  },
  {
    player_id: "p2",
    display_name: "Beta",
    total_points: -20,
    health: {
      score: 47.2,
      grade: "D",
      level: "red" as const,
      trend: null,
      verified: false,
      points: 1,
    },
  },
  { player_id: "p3", display_name: "Gamma", total_points: 0, health: null },
];

describe("ScreenLeaderboard", () => {
  it("shows each player's score and current code health, no Arena Points", () => {
    render(ScreenLeaderboard, { entries: rows, emptyMessage: "—" });
    const items = screen.getByTestId("screen-leaderboard").querySelectorAll("li");
    expect(items).toHaveLength(3);
    const badges = screen.getAllByTestId("screen-health");
    expect(badges).toHaveLength(2);
    expect(badges[0].getAttribute("data-level")).toBe("green");
    expect(badges[0].textContent).toContain("B");
    expect(badges[0].textContent).toContain("78");
    expect(badges[0].textContent).toContain("▲");
    expect(badges[1].getAttribute("data-level")).toBe("red");
    expect(items[2].textContent).not.toContain("AP");
    expect(screen.getByTestId("screen-leaderboard").textContent).not.toContain("AP");
  });

  it("names an empty board", () => {
    render(ScreenLeaderboard, { entries: [], emptyMessage: "Waiting for first results…" });
    expect(screen.getByText("Waiting for first results…")).toBeTruthy();
  });
});
