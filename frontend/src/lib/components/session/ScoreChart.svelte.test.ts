import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/svelte";
import ScoreChart from "./ScoreChart.svelte";
import type { LeaderboardEntry, MemberInfo, ScoreHistoryPoint } from "$lib/types/arena";
import type { SessionReportResponse, SessionReportTimelineEntry } from "$lib/api";

const ALICE_MEMBER: MemberInfo = {
  user_id: "p1",
  display_name: "Alice",
  joined_at: "",
  avatar_url: null,
  fingerprint: null,
  username: null,
  agent_display_name: null,
};

const ALICE_ENTRY: LeaderboardEntry = {
  player_id: "p1",
  display_name: "Alice",
  agent_display_name: null,
  total_points: 5,
  tests_passed: 1,
  total_wall_ms: 0,
};

describe("ScoreChart", () => {
  it("finished phase with populated scoreHistory shows no empty message", () => {
    const scoreHistory: ScoreHistoryPoint[] = [
      { t: 0, scores: { p1: 0 } },
      { t: 10, scores: { p1: 5 } },
    ];

    render(ScoreChart, {
      phase: "finished",
      leaderboard: [],
      reportMembers: [ALICE_MEMBER],
      sessionReport: null,
      scoreHistory,
    });

    expect(screen.queryByText("No score history recorded.")).toBeNull();
  });

  it("active phase with populated scoreHistory shows no empty message", () => {
    const scoreHistory: ScoreHistoryPoint[] = [{ t: 0, scores: { p1: 5 } }];

    render(ScoreChart, {
      phase: "active",
      leaderboard: [ALICE_ENTRY],
      reportMembers: [],
      sessionReport: null,
      scoreHistory,
    });

    expect(screen.queryByText("Waiting for first scores…")).toBeNull();
  });

  it('finished phase with empty scoreHistory shows "No score history recorded."', () => {
    render(ScoreChart, {
      phase: "finished",
      leaderboard: [],
      reportMembers: [ALICE_MEMBER],
      sessionReport: null,
      scoreHistory: [],
    });

    expect(screen.queryByText("No score history recorded.")).not.toBeNull();
  });

  it('active phase with empty scoreHistory shows "Waiting for first scores…"', () => {
    render(ScoreChart, {
      phase: "active",
      leaderboard: [ALICE_ENTRY],
      reportMembers: [],
      sessionReport: null,
      scoreHistory: [],
    });

    expect(screen.queryByText("Waiting for first scores…")).not.toBeNull();
  });

  it("does not read sessionReport.timeline for the chart", () => {
    const timeline: SessionReportTimelineEntry[] = [
      {
        task_id: "t1",
        task_title: "Task 1",
        player_id: "p1",
        player_display_name: "Alice",
        score: 5,
        answer: "42",
        created_at: "2026-01-01T00:00:00Z",
      },
    ];
    const sessionReport: SessionReportResponse = {
      session_id: "s1",
      status: "finished",
      leaderboard: [],
      timeline,
      activity_events: [],
    };

    render(ScoreChart, {
      phase: "finished",
      leaderboard: [],
      reportMembers: [],
      sessionReport,
      scoreHistory: [],
    });

    // scoreHistory is empty — chart must show the empty message regardless of
    // sessionReport.timeline contents. Guards against re-introducing the
    // REST timeline fold that previously fed the chart from sessionReport.
    expect(screen.queryByText("No score history recorded.")).not.toBeNull();
  });
});
