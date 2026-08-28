import { describe, expect, it } from "vitest";
import { countdownDigits } from "./format";
import {
  buildSessionTimeline,
  getAvailableSessionScreens,
  getDefaultSessionScreen,
  resolvePlayerTaskStates,
  type PlayerProgressState,
} from "./session-view-model";

describe("session-view-model", () => {
  it("picks lobby as default screen for lobby sessions", () => {
    expect(getDefaultSessionScreen("lobby")).toBe("lobby");
  });

  it("picks dashboard as default screen for running sessions", () => {
    expect(getDefaultSessionScreen("running")).toBe("dashboard");
  });

  it("picks report as default screen for finished sessions", () => {
    expect(getDefaultSessionScreen("finished")).toBe("report");
  });

  it("formats countdown as hh:mm:ss", () => {
    expect(countdownDigits(3661)).toEqual(["01", "01", "01"]);
  });

  it("resolves player task state from progress", () => {
    const progress: PlayerProgressState = {
      current_task_id: "task-2",
      visited_task_ids: ["task-1"],
    };

    expect(resolvePlayerTaskStates(progress, "task-1")).toBe("completed");
    expect(resolvePlayerTaskStates(progress, "task-2")).toBe("active");
    expect(resolvePlayerTaskStates(progress, "task-3")).toBe("pending");
  });

  it("limits available screens by phase", () => {
    expect(getAvailableSessionScreens("lobby")).toEqual(["lobby"]);
    expect(getAvailableSessionScreens("active")).toEqual(["dashboard", "player"]);
    expect(getAvailableSessionScreens("finished")).toEqual(["report", "dashboard"]);
  });

  it("builds timeline from score history", () => {
    const timeline = buildSessionTimeline(
      [
        { t: 5, scores: { p1: 1 } },
        { t: 10, scores: { p1: 2 } },
      ],
      [
        {
          player_id: "p1",
          display_name: "Alpha",
          total_points: 20,
          tests_passed: 2,
          total_wall_ms: 100,
        },
      ],
    );

    expect(timeline).toHaveLength(2);
    expect(timeline[1].score).toBe(2);
    expect(timeline[1].player_name).toBe("Alpha");
  });

  it("builds fallback timeline from leaderboard when history is empty", () => {
    const timeline = buildSessionTimeline(
      [],
      [
        {
          player_id: "p1",
          display_name: "Alpha",
          total_points: 30,
          tests_passed: 3,
          total_wall_ms: 100,
        },
      ],
    );
    expect(timeline).toHaveLength(1);
    expect(timeline[0].at_seconds).toBe(0);
    expect(timeline[0].score).toBe(30);
  });
});
