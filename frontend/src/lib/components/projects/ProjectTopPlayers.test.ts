import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { tick } from "svelte";
import ProjectTopPlayers from "./ProjectTopPlayers.svelte";
import type { TopPlayer } from "$lib/api";

function player(name: string, points: number, rank: number): TopPlayer {
  return {
    rank,
    user_id: `${name}-id`,
    username: name,
    display_name: name,
    avatar_url: null,
    game_points: points,
    sessions_played: 2,
    best_placement: 1,
  };
}

const allTime = [player("Ada", 900, 1), player("Grace", 400, 2)];
const seasonOnly = [player("Grace", 400, 1)];
const AUGUST = "2026-08-01T00:00:00Z";

describe("ProjectTopPlayers", () => {
  it("Shows the all-time board first, since a season resets to empty", () => {
    render(ProjectTopPlayers, {
      players: allTime,
      seasonPlayers: seasonOnly,
      seasonStart: AUGUST,
    });
    expect(screen.getByTestId("top-players-all").getAttribute("aria-selected")).toBe("true");
    expect(screen.getByText("Ada")).not.toBeNull();
  });

  it("Labels the seasonal tab with the current month and its own count", () => {
    render(ProjectTopPlayers, {
      players: allTime,
      seasonPlayers: seasonOnly,
      seasonStart: AUGUST,
    });
    const seasonTab = screen.getByTestId("top-players-season");
    expect(seasonTab.textContent).toContain("August");
    expect(seasonTab.textContent).toContain("(1)");
    expect(screen.getByTestId("top-players-all").textContent).toContain("(2)");
  });

  it("Switches to the season board, dropping players who only ranked earlier", async () => {
    render(ProjectTopPlayers, {
      players: allTime,
      seasonPlayers: seasonOnly,
      seasonStart: AUGUST,
    });
    await fireEvent.click(screen.getByTestId("top-players-season"));
    await tick();
    expect(screen.queryByText("Ada")).toBeNull();
    expect(screen.getByText("Grace")).not.toBeNull();
  });

  it("Explains an empty season instead of implying the project is dead", async () => {
    render(ProjectTopPlayers, {
      players: allTime,
      seasonPlayers: [],
      seasonStart: AUGUST,
    });
    await fireEvent.click(screen.getByTestId("top-players-season"));
    await tick();
    expect(screen.getByTestId("top-players-empty").textContent).toContain(
      "the all-time board still stands",
    );
  });

  it("Falls back to the never-played copy when the project has no history at all", () => {
    render(ProjectTopPlayers, { players: [], seasonPlayers: [], seasonStart: AUGUST });
    expect(screen.getByTestId("top-players-empty").textContent).toContain("No players yet");
  });

  it("Says how far each player got through a campaign, and marks the finishers", () => {
    const finisher = { ...player("Ada", 900, 1), parts_completed: 5 };
    const halfway = { ...player("Grace", 400, 2), parts_completed: 2 };
    render(ProjectTopPlayers, {
      players: [finisher, halfway],
      seasonPlayers: [],
      seasonStart: AUGUST,
      partsTotal: 5,
    });
    expect(screen.getByTestId("campaign-progress-Ada-id").textContent).toContain(
      "Campaign complete",
    );
    expect(screen.getByTestId("campaign-progress-Ada-id").textContent).toContain("🏁");
    const behind = screen.getByTestId("campaign-progress-Grace-id");
    expect(behind.textContent).toContain("2 of 5 parts done");
    expect(behind.textContent).not.toContain("🏁");
  });

  it("Says nothing about parts on an ordinary project board", () => {
    render(ProjectTopPlayers, {
      players: allTime,
      seasonPlayers: [],
      seasonStart: AUGUST,
    });
    expect(screen.queryByTestId("campaign-progress-Ada-id")).toBeNull();
  });

  it("Survives a missing season_start (the loader's failure fallback)", () => {
    render(ProjectTopPlayers, { players: allTime, seasonPlayers: [], seasonStart: "" });
    // Falls back to the current month rather than rendering "Invalid Date".
    const label = screen.getByTestId("top-players-season").textContent ?? "";
    expect(label).not.toContain("Invalid");
  });
});
