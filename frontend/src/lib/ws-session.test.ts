import { describe, expect, it } from "vitest";
import { WsSessionClient } from "./ws-session.svelte";

describe("WsSessionClient frame mapping", () => {
  function emit(client: WsSessionClient, frame: unknown) {
    (client as unknown as { _handleFrame: (frame: unknown) => void })._handleFrame(frame);
  }

  it("updates leaderboard from player_id entries", () => {
    const client = new WsSessionClient("ABC123", null);

    emit(client, {
      type: "leaderboard_update",
      entries: [
        {
          player_id: "player-1",
          display_name: "Player One",
          tests_passed: 3,
          total_wall_ms: 250,
        },
      ],
    });

    expect(client.leaderboard).toHaveLength(1);
    expect(client.leaderboard[0].player_id).toBe("player-1");
  });

  it("tracks player progress from player_progress_update", () => {
    const client = new WsSessionClient("ABC123", null);

    emit(client, {
      type: "player_progress_update",
      player_id: "player-2",
      current_task_id: "task-9",
      attempt: 1,
      status: "awaiting_result",
    });

    expect(client.agentProgress["player-2"]?.current_task_id).toBe("task-9");
  });

  it("removes disconnected player from participants", () => {
    const client = new WsSessionClient("ABC123", null);

    emit(client, {
      type: "member_joined",
      user_id: "player-3",
      display_name: "Player Three",
      joined_at: "2026-05-21T00:00:00Z",
    });

    emit(client, {
      type: "player_disconnected",
      player_id: "player-3",
    });

    expect(client.participants).toHaveLength(0);
  });

  it("ignores stale phase frames when version is older", () => {
    const client = new WsSessionClient("ABC123", null);

    emit(client, {
      type: "session_snapshot",
      session_id: "sess-1",
      phase: "active",
      version: 10,
      participants: [],
      leaderboard: [],
      started_at: null,
    });

    emit(client, {
      type: "session_started",
      session_id: "sess-1",
      version: 8,
    });

    expect(client.phase).toBe("active");
  });

  it("initializes userPlayers as empty array", () => {
    const client = new WsSessionClient("ABC123", null);
    expect(client.userPlayers).toEqual([]);
  });

  it("sets phase to paused when session_snapshot has paused phase", () => {
    const client = new WsSessionClient("ABC123", null);

    emit(client, {
      type: "session_snapshot",
      session_id: "sess-1",
      phase: "paused",
      version: 12,
      participants: [],
      leaderboard: [],
      started_at: null,
    });

    expect(client.phase).toBe("paused");
    expect(client.isPaused).toBe(true);
  });

  it("exposes isPaused as false for active phase", () => {
    const client = new WsSessionClient("ABC123", null);

    emit(client, {
      type: "session_started",
      session_id: "sess-1",
      version: 1,
    });

    expect(client.phase).toBe("active");
    expect(client.isPaused).toBe(false);
  });

  it("sets userPlayers from user_players_snapshot frame and does not set protocolMismatch", () => {
    const client = new WsSessionClient("ABC123", null);

    emit(client, {
      type: "user_players_snapshot",
      players: [
        {
          player_id: "player-1",
          display_name: "Alice",
          fingerprint: "abc123",
          joined_at: "2026-05-22T00:00:00Z",
          reconnected_at: null,
          revoked_at: null,
        },
        {
          player_id: "player-2",
          display_name: "Bob",
          fingerprint: null,
          joined_at: "2026-05-22T00:01:00Z",
          reconnected_at: null,
          revoked_at: null,
        },
      ],
    });

    expect(client.userPlayers).toHaveLength(2);
    expect(client.userPlayers[0].player_id).toBe("player-1");
    expect(client.userPlayers[1].display_name).toBe("Bob");
    expect(client.protocolMismatch).toBe(false);
  });

  it("keeps latest participant snapshot under out-of-order delivery", () => {
    const client = new WsSessionClient("ABC123", null);

    emit(client, {
      type: "session_snapshot",
      session_id: "sess-1",
      phase: "lobby",
      version: 5,
      participants: [
        {
          user_id: "u1",
          display_name: "Alpha",
          joined_at: "2026-05-22T00:00:00Z",
          avatar_url: "https://example.com/a.png",
          fingerprint: "0123456789abcdef",
        },
      ],
      leaderboard: [],
      started_at: null,
    });

    emit(client, {
      type: "member_joined",
      user_id: "u2",
      display_name: "Beta",
      joined_at: "2026-05-22T00:00:01Z",
      avatar_url: null,
      fingerprint: "feedfacecafebeef",
      version: 4,
    });

    expect(client.participants).toHaveLength(1);
    expect(client.participants[0].user_id).toBe("u1");
  });
});
