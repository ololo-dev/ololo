import { beforeEach, describe, expect, it, vi } from "vitest";

const captured: { path: string; onMessage: (raw: string) => void }[] = [];

vi.mock("$lib/ws/connection.svelte", () => ({
  createWsConnection: vi.fn((opts: { path: string; onMessage: (raw: string) => void }) => {
    captured.push({ path: opts.path, onMessage: opts.onMessage });
    return { connect: vi.fn(), disconnect: vi.fn(), connected: false };
  }),
}));

import { WsSessionClient } from "./ws-session.svelte";

function client(token: string | null = null) {
  const c = new WsSessionClient("CODE", token);
  return c;
}

describe("WsSessionClient", () => {
  beforeEach(() => {
    captured.length = 0;
  });

  it("builds an authenticated path with token", () => {
    client("jwt-token");
    expect(captured[0].path).toBe("/ws/s/CODE?token=jwt-token");
  });

  it("builds an observe path without a token", () => {
    client(null);
    expect(captured[0].path).toBe("/ws/s/CODE/observe");
  });

  it("delegates connect and disconnect", () => {
    const c = client();
    c.connect();
    c.disconnect();
    expect(captured.length).toBe(1);
  });

  it("flags protocolMismatch on malformed JSON", () => {
    const c = client();
    captured[0].onMessage("}{");
    expect(c.protocolMismatch).toBe(true);
  });

  it("flags protocolMismatch on unknown frame type", () => {
    const c = client();
    captured[0].onMessage(JSON.stringify({ type: "nope" }));
    expect(c.protocolMismatch).toBe(true);
  });

  it("ignores frames with an older or equal version", () => {
    const c = client();
    captured[0].onMessage(
      JSON.stringify({ type: "lobby_countdown", version: 5, seconds_remaining: 3 }),
    );
    captured[0].onMessage(
      JSON.stringify({ type: "lobby_countdown", version: 5, seconds_remaining: 9 }),
    );
    expect(c.countdownSecs).toBe(3);
  });

  it("processes newer versions", () => {
    const c = client();
    captured[0].onMessage(
      JSON.stringify({ type: "lobby_countdown", version: 1, seconds_remaining: 3 }),
    );
    captured[0].onMessage(
      JSON.stringify({ type: "lobby_countdown", version: 2, seconds_remaining: 9 }),
    );
    expect(c.countdownSecs).toBe(9);
  });

  it("tracks lobby and running countdowns", () => {
    const c = client();
    captured[0].onMessage(JSON.stringify({ type: "lobby_countdown", seconds_remaining: 7 }));
    captured[0].onMessage(JSON.stringify({ type: "running_countdown", seconds_remaining: 4 }));
    expect(c.countdownSecs).toBe(7);
    expect(c.runningCountdownSecs).toBe(4);
  });

  it("advances to active on session_started", () => {
    const c = client();
    captured[0].onMessage(JSON.stringify({ type: "session_started" }));
    expect(c.phase).toBe("active");
    expect(c.startedAt).not.toBeNull();
    expect(c.countdownSecs).toBeNull();
  });

  it("finishes on session_complete", () => {
    const c = client();
    captured[0].onMessage(JSON.stringify({ type: "session_complete" }));
    expect(c.phase).toBe("finished");
    expect(c.runningCountdownSecs).toBeNull();
  });

  it("maps leaderboard_update entries and records score history", () => {
    const c = client();
    captured[0].onMessage(
      JSON.stringify({
        type: "leaderboard_update",
        entries: [{ agent_id: "a1", total_points: 10 }],
      }),
    );
    expect(c.leaderboard).toEqual([{ agent_id: "a1", player_id: "a1", total_points: 10 }]);
    expect(c.scoreHistory.length).toBe(1);
    expect(c.scoreHistory[0].scores).toEqual({ a1: 10 });
  });

  it("accumulates visited task ids on player progress updates", () => {
    const c = client();
    captured[0].onMessage(
      JSON.stringify({ type: "player_progress_update", player_id: "p1", current_task_id: "t1" }),
    );
    captured[0].onMessage(
      JSON.stringify({ type: "player_progress_update", player_id: "p1", current_task_id: "t2" }),
    );
    expect(c.agentProgress.p1).toEqual({ current_task_id: "t2", visited_task_ids: ["t1"] });
  });

  it("accumulates visited task ids on agent progress updates", () => {
    const c = client();
    captured[0].onMessage(
      JSON.stringify({ type: "agent_progress_update", agent_id: "a1", current_task_id: "t1" }),
    );
    captured[0].onMessage(
      JSON.stringify({ type: "agent_progress_update", agent_id: "a1", current_task_id: "t2" }),
    );
    expect(c.agentProgress.a1).toEqual({ current_task_id: "t2", visited_task_ids: ["t1"] });
  });

  it("applies a session snapshot and seeds score history once", () => {
    const c = client();
    captured[0].onMessage(
      JSON.stringify({
        type: "session_snapshot",
        version: 1,
        phase: "running",
        participants: [{ user_id: "p1" }],
        leaderboard: [{ player_id: "p1", total_points: 5 }],
        started_at: 1000,
      }),
    );
    expect(c.phase).toBe("active");
    expect(c.participants).toEqual([{ user_id: "p1" }]);
    expect(c.leaderboard).toEqual([{ player_id: "p1", total_points: 5 }]);
    expect(c.startedAt).toEqual(new Date(1000));
    expect(c.scoreHistory.length).toBe(1);
    // Re-sending the same snapshot does not append more history.
    captured[0].onMessage(
      JSON.stringify({
        type: "session_snapshot",
        version: 2,
        phase: "running",
        participants: [{ user_id: "p1" }],
        leaderboard: [{ player_id: "p1", total_points: 5 }],
        started_at: 1000,
      }),
    );
    expect(c.scoreHistory.length).toBe(1);
  });

  it("applies a session snapshot with score_history and seeds the full series", () => {
    const c = client();
    captured[0].onMessage(
      JSON.stringify({
        type: "session_snapshot",
        version: 1,
        phase: "running",
        participants: [{ user_id: "p1" }, { user_id: "p2" }],
        leaderboard: [
          { player_id: "p1", total_points: 8 },
          { player_id: "p2", total_points: 3 },
        ],
        started_at: 1000,
        score_history: [
          { t: 0, scores: { p1: 5 } },
          { t: 10, scores: { p1: 5, p2: 3 } },
          { t: 20, scores: { p1: 8, p2: 3 } },
        ],
      }),
    );
    expect(c.scoreHistory.length).toBe(3);
    expect(c.scoreHistory[2].scores.p1).toBe(8);
  });

  it("applies a finished-phase session snapshot with score_history and seeds the full series", () => {
    const c = client();
    captured[0].onMessage(
      JSON.stringify({
        type: "session_snapshot",
        version: 1,
        phase: "finished",
        participants: [{ user_id: "p1" }],
        leaderboard: [{ player_id: "p1", total_points: 10 }],
        started_at: 1000,
        score_history: [
          { t: 0, scores: { p1: 5 } },
          { t: 30, scores: { p1: 10 } },
        ],
      }),
    );
    expect(c.scoreHistory.length).toBe(2);
    expect(c.scoreHistory[1].scores.p1).toBe(10);
  });

  it("exposes participant completion_status from a session snapshot; absent stays undefined", () => {
    const c = client();
    captured[0].onMessage(
      JSON.stringify({
        type: "session_snapshot",
        version: 1,
        phase: "running",
        participants: [{ user_id: "p1", completion_status: "awaiting_judges" }, { user_id: "p2" }],
        leaderboard: [],
        started_at: null,
      }),
    );
    expect(c.participants[0].completion_status).toBe("awaiting_judges");
    expect(c.participants[1].completion_status).toBeUndefined();
  });

  it("removes participants on player_disconnected", () => {
    const c = client();
    captured[0].onMessage(
      JSON.stringify({
        type: "session_snapshot",
        phase: "lobby",
        participants: [{ user_id: "p1" }, { user_id: "p2" }],
      }),
    );
    captured[0].onMessage(JSON.stringify({ type: "player_disconnected", player_id: "p1" }));
    expect(c.participants.map((p) => p.user_id)).toEqual(["p2"]);
  });

  it("removes participants on member_disconnected", () => {
    const c = client();
    captured[0].onMessage(
      JSON.stringify({
        type: "session_snapshot",
        phase: "lobby",
        participants: [{ user_id: "p1" }, { user_id: "p2" }],
      }),
    );
    captured[0].onMessage(JSON.stringify({ type: "member_disconnected", agent_id: "p2" }));
    expect(c.participants.map((p) => p.user_id)).toEqual(["p1"]);
  });

  it("adds a member on member_joined", () => {
    const c = client();
    captured[0].onMessage(
      JSON.stringify({ type: "member_joined", user_id: "p1", display_name: "A", joined_at: "now" }),
    );
    expect(c.participants).toEqual([
      {
        user_id: "p1",
        player_id: null,
        display_name: "A",
        joined_at: "now",
        avatar_url: null,
        fingerprint: null,
        username: null,
        agent_display_name: null,
      },
    ]);
  });

  it("updates an existing member on member_joined", () => {
    const c = client();
    captured[0].onMessage(
      JSON.stringify({ type: "member_joined", user_id: "p1", display_name: "A", joined_at: "now" }),
    );
    captured[0].onMessage(
      JSON.stringify({
        type: "member_joined",
        user_id: "p1",
        display_name: "B",
        joined_at: "later",
        avatar_url: "u",
      }),
    );
    expect(c.participants[0]).toMatchObject({
      display_name: "B",
      avatar_url: "u",
      joined_at: "later",
    });
  });

  it("ignores heartbeat frames", () => {
    const c = client();
    captured[0].onMessage(JSON.stringify({ type: "heartbeat" }));
    expect(c.protocolMismatch).toBe(false);
  });

  it("stores user players on user_players_snapshot", () => {
    const c = client();
    captured[0].onMessage(
      JSON.stringify({ type: "user_players_snapshot", players: [{ player_id: "p1" }] }),
    );
    expect(c.userPlayers).toEqual([{ player_id: "p1" }]);
  });

  it("updates phase on project_session_update with status", () => {
    const c = client();
    captured[0].onMessage(JSON.stringify({ type: "project_session_update", status: "cancelled" }));
    expect(c.phase).toBe("finished");
  });

  it("seeds activityLog from session_snapshot activity field", () => {
    const c = client();
    captured[0].onMessage(
      JSON.stringify({
        type: "session_snapshot",
        session_id: "s1",
        phase: "running",
        version: 1,
        participants: [],
        leaderboard: [],
        started_at: null,
        activity: [
          {
            event_kind: "task_started",
            player_id: "p1",
            player_display_name: "Alpha",
            task_id: "t1",
            task_ordinal: 1,
            task_title: "Task One",
            judge_name: null,
            point_delta: null,
            timestamp: "2026-07-16T00:00:00Z",
            version: 1,
          },
        ],
      }),
    );
    expect(c.activityLog).toEqual([
      {
        kind: "task_started",
        player_id: "p1",
        player_display_name: "Alpha",
        task_id: "t1",
        task_ordinal: 1,
        task_title: "Task One",
        judge_name: null,
        point_delta: null,
        detail: null,
        timestamp: "2026-07-16T00:00:00Z",
        version: 1,
      },
    ]);
  });

  it("appends live task_started events after snapshot-seeded activityLog", () => {
    const c = client();
    captured[0].onMessage(
      JSON.stringify({
        type: "session_snapshot",
        session_id: "s1",
        phase: "running",
        version: 1,
        participants: [],
        leaderboard: [],
        started_at: null,
        activity: [
          {
            event_kind: "task_started",
            player_id: "p1",
            player_display_name: "Alpha",
            task_id: "t1",
            task_ordinal: 1,
            task_title: "Task One",
            judge_name: null,
            point_delta: null,
            timestamp: "2026-07-16T00:00:00Z",
            version: 1,
          },
        ],
      }),
    );
    captured[0].onMessage(
      JSON.stringify({
        type: "task_scored",
        player_id: "p2",
        player_display_name: "Beta",
        task_id: "t2",
        task_ordinal: 2,
        task_title: "Task Two",
        point_delta: 10,
        judge_name: "AI Judge",
        timestamp: "2026-07-16T00:01:00Z",
        version: 2,
      }),
    );
    expect(c.activityLog).toEqual([
      {
        kind: "task_started",
        player_id: "p1",
        player_display_name: "Alpha",
        task_id: "t1",
        task_ordinal: 1,
        task_title: "Task One",
        judge_name: null,
        point_delta: null,
        detail: null,
        timestamp: "2026-07-16T00:00:00Z",
        version: 1,
      },
      {
        kind: "task_scored",
        player_id: "p2",
        player_display_name: "Beta",
        task_id: "t2",
        task_ordinal: 2,
        task_title: "Task Two",
        judge_name: "AI Judge",
        point_delta: 10,
        detail: null,
        timestamp: "2026-07-16T00:01:00Z",
        version: 2,
      },
    ]);
  });

  it("drops duplicate task_started events for the same player and task", () => {
    const c = client();
    const frame = {
      type: "task_started",
      player_id: "p1",
      player_display_name: "Alpha",
      task_id: "t1",
      task_ordinal: 1,
      task_title: "Task One",
      timestamp: "2026-07-16T00:00:00Z",
      version: 1,
    };
    captured[0].onMessage(JSON.stringify(frame));
    captured[0].onMessage(
      JSON.stringify({ ...frame, timestamp: "2026-07-16T00:01:00Z", version: 2 }),
    );
    captured[0].onMessage(
      JSON.stringify({ ...frame, player_id: "p2", player_display_name: "Beta" }),
    );
    expect(c.activityLog.length).toBe(2);
    expect(c.activityLog[0].player_id).toBe("p1");
    expect(c.activityLog[0].timestamp).toBe("2026-07-16T00:00:00Z");
    expect(c.activityLog[1].player_id).toBe("p2");
  });

  it("appends task_started events to activityLog", () => {
    const c = client();
    captured[0].onMessage(
      JSON.stringify({
        type: "task_started",
        player_id: "p1",
        player_display_name: "Alpha",
        task_id: "t1",
        task_ordinal: 1,
        task_title: "Task One",
        timestamp: "2026-07-16T00:00:00Z",
        version: 1,
      }),
    );
    expect(c.activityLog).toEqual([
      {
        kind: "task_started",
        player_id: "p1",
        player_display_name: "Alpha",
        task_id: "t1",
        task_ordinal: 1,
        task_title: "Task One",
        judge_name: null,
        point_delta: null,
        timestamp: "2026-07-16T00:00:00Z",
        version: 1,
      },
    ]);
  });

  it("appends task_scored events to activityLog", () => {
    const c = client();
    captured[0].onMessage(
      JSON.stringify({
        type: "task_scored",
        player_id: "p2",
        player_display_name: "Beta",
        task_id: "t2",
        task_ordinal: 2,
        task_title: "Task Two",
        point_delta: 10,
        judge_name: "AI Judge",
        timestamp: "2026-07-16T00:01:00Z",
        version: 2,
      }),
    );
    expect(c.activityLog).toEqual([
      {
        kind: "task_scored",
        player_id: "p2",
        player_display_name: "Beta",
        task_id: "t2",
        task_ordinal: 2,
        task_title: "Task Two",
        judge_name: "AI Judge",
        point_delta: 10,
        detail: null,
        timestamp: "2026-07-16T00:01:00Z",
        version: 2,
      },
    ]);
  });
  it("collapses repeated implemented rows but keeps each judge verdict", () => {
    const c = client();
    const scored = (over: Record<string, unknown>) =>
      JSON.stringify({
        type: "task_scored",
        player_id: "p1",
        player_display_name: "Alpha",
        task_id: "t0",
        task_ordinal: 0,
        task_title: "Task Zero",
        judge_name: null,
        point_delta: 20,
        timestamp: "2026-07-16T00:00:00Z",
        version: 1,
        ...over,
      });
    // Three probe-pass "implemented" scores on the same task → one feed row.
    captured[0].onMessage(scored({}));
    captured[0].onMessage(scored({ timestamp: "2026-07-16T00:00:30Z" }));
    captured[0].onMessage(scored({ timestamp: "2026-07-16T00:01:00Z" }));
    // Two distinct judge verdicts on that task → both kept.
    captured[0].onMessage(scored({ judge_name: "anti-cheater", point_delta: 0 }));
    captured[0].onMessage(scored({ judge_name: "code-cleanliness", point_delta: -3 }));
    // A re-run of anti-cheater (recovery sweep / re-judge) replaces its prior
    // verdict rather than stacking a duplicate; its point_delta updates.
    captured[0].onMessage(scored({ judge_name: "anti-cheater", point_delta: -10 }));

    const implemented = c.activityLog.filter((e) => e.kind === "task_scored" && !e.judge_name);
    const verdicts = c.activityLog.filter((e) => e.kind === "task_scored" && e.judge_name);
    expect(implemented.length).toBe(1);
    expect(verdicts.map((v) => v.judge_name)).toEqual(["anti-cheater", "code-cleanliness"]);
    expect(verdicts.find((v) => v.judge_name === "anti-cheater")?.point_delta).toBe(-10);
  });

  it("keeps the same judge's verdict on each distinct task", () => {
    const c = client();
    const verdict = (taskId: string, ordinal: number) =>
      JSON.stringify({
        type: "task_scored",
        player_id: "p1",
        player_display_name: "Alpha",
        task_id: taskId,
        task_ordinal: ordinal,
        task_title: `Task ${ordinal}`,
        judge_name: "golf-verify",
        point_delta: 0,
        timestamp: "2026-07-16T00:00:00Z",
        version: 1,
      });
    // golf-verify runs once per task — three tasks, three distinct rows.
    captured[0].onMessage(verdict("t0", 0));
    captured[0].onMessage(verdict("t1", 1));
    captured[0].onMessage(verdict("t2", 2));
    const verdicts = c.activityLog.filter((e) => e.kind === "task_scored" && e.judge_name);
    expect(verdicts.map((v) => v.task_id)).toEqual(["t0", "t1", "t2"]);
  });
});
