import type { LeaderboardEntry, MemberInfo, PlayerSummary } from "$lib/types/arena";

interface MergeLeaderboardArgs {
  leaderboard: LeaderboardEntry[];
  participants: MemberInfo[];
  userPlayers: PlayerSummary[];
}

export function mergeLeaderboardEntries({
  leaderboard,
  participants,
  userPlayers,
}: MergeLeaderboardArgs): LeaderboardEntry[] {
  const ownUserIdToPlayerId = new Map(
    userPlayers
      .filter((player) => player.user_id !== null)
      .map((player) => [player.user_id as string, player.player_id]),
  );

  const listedPlayerIds = new Set<string>();
  const merged: LeaderboardEntry[] = [];

  const pushIfMissing = (entry: LeaderboardEntry) => {
    if (listedPlayerIds.has(entry.player_id)) return;
    listedPlayerIds.add(entry.player_id);
    merged.push(entry);
  };

  for (const entry of leaderboard) {
    const playerId = ownUserIdToPlayerId.get(entry.player_id) ?? entry.player_id;
    pushIfMissing({ ...entry, player_id: playerId });
  }

  for (const participant of participants) {
    // Prefer the participant's own player id (authoritative, present on
    // newer frames); fall back to the viewer's own-player map, then user_id.
    const playerId =
      participant.player_id ?? ownUserIdToPlayerId.get(participant.user_id) ?? participant.user_id;
    pushIfMissing({
      player_id: playerId,
      display_name: participant.display_name,
      total_points: 0,
      tests_passed: 0,
      total_wall_ms: 0,
    });
  }

  for (const player of userPlayers) {
    pushIfMissing({
      player_id: player.player_id,
      display_name: player.display_name,
      total_points: 0,
      tests_passed: 0,
      total_wall_ms: 0,
    });
  }

  return merged.sort(
    (a, b) =>
      b.total_points - a.total_points ||
      b.tests_passed - a.tests_passed ||
      a.display_name.localeCompare(b.display_name),
  );
}
