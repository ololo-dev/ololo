import { describe, expect, it } from "vitest";
import { mergeLeaderboardEntries } from "$lib/session-leaderboard";

describe("mergeLeaderboardEntries", () => {
  it("deduplicates own row when scored entry uses user_id", () => {
    const userId = "user-1";
    const playerId = "player-1";

    const merged = mergeLeaderboardEntries({
      leaderboard: [
        {
          player_id: userId,
          display_name: "Me",
          total_points: 30,
          tests_passed: 3,
          total_wall_ms: 100,
        },
      ],
      participants: [
        {
          user_id: userId,
          display_name: "Me",
          joined_at: "2026-01-01T00:00:00Z",
          avatar_url: "https://example.com/me.png",
          fingerprint: "abc",
          username: "me",
        },
      ],
      userPlayers: [
        {
          player_id: playerId,
          user_id: userId,
          display_name: "Me",
          fingerprint: null,
          joined_at: "2026-01-01T00:00:00Z",
          reconnected_at: null,
          revoked_at: null,
        },
      ],
    });

    expect(merged).toHaveLength(1);
    expect(merged[0]?.player_id).toBe(playerId);
    expect(merged[0]?.tests_passed).toBe(3);
  });

  it("dedupes a scored player against its participant entry via player_id (no userPlayers)", () => {
    // Regression: an account-linked player appeared twice for any viewer who
    // didn't own it — once from the leaderboard (keyed by player_id) and once
    // from participants (keyed by user_id) — because the ids never matched.
    const merged = mergeLeaderboardEntries({
      leaderboard: [
        {
          player_id: "player-1",
          display_name: "Andrey",
          total_points: 120,
          tests_passed: 3,
          total_wall_ms: 0,
        },
      ],
      participants: [
        {
          user_id: "user-1",
          player_id: "player-1",
          display_name: "Andrey",
          joined_at: "2026-07-21T10:00:00Z",
          avatar_url: "https://example.com/a.png",
          fingerprint: null,
          username: "andrey",
        },
      ],
      userPlayers: [],
    });

    expect(merged).toHaveLength(1);
    expect(merged[0]?.player_id).toBe("player-1");
    expect(merged[0]?.total_points).toBe(120);
  });

  it("still lists not-yet-scored participants keyed by their player_id", () => {
    const merged = mergeLeaderboardEntries({
      leaderboard: [],
      participants: [
        {
          user_id: "user-2",
          player_id: "player-2",
          display_name: "Beta",
          joined_at: "2026-07-21T10:00:00Z",
          avatar_url: null,
          fingerprint: null,
          username: "beta",
        },
      ],
      userPlayers: [],
    });

    expect(merged).toHaveLength(1);
    expect(merged[0]?.player_id).toBe("player-2");
    expect(merged[0]?.total_points).toBe(0);
  });
});
