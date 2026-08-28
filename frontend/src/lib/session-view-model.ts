import type { LeaderboardEntry, ScoreHistoryPoint } from "$lib/types/arena";

export type SessionScreen = "lobby" | "dashboard" | "player" | "report";
export type PlayerTaskState = "pending" | "active" | "completed";

export interface TimelinePoint {
  at_seconds: number;
  player_id: string;
  player_name: string;
  score: number;
}

export interface PlayerProgressState {
  current_task_id: string | null;
  visited_task_ids: string[];
}

// ponytail: PlayerProgressState is the canonical progress shape; AgentProgressState
// (in ws-session.svelte.ts) is kept as a type alias for backward compatibility.

export function getDefaultSessionScreen(sessionState: string): SessionScreen {
  if (sessionState === "running") return "dashboard";
  if (sessionState === "finished" || sessionState === "cancelled") return "report";
  return "lobby";
}

export function getAvailableSessionScreens(phase: string): SessionScreen[] {
  if (phase === "lobby") return ["lobby"];
  if (phase === "active") return ["dashboard", "player"];
  return ["report", "dashboard"];
}

export function resolvePlayerTaskStates(
  progress: PlayerProgressState | undefined,
  taskId: string,
): PlayerTaskState {
  if (!progress) return "pending";
  if (progress.current_task_id === taskId) return "active";
  if (progress.visited_task_ids.includes(taskId)) return "completed";
  return "pending";
}

export function buildSessionTimeline(
  scoreHistory: ScoreHistoryPoint[],
  leaderboard: LeaderboardEntry[],
): TimelinePoint[] {
  if (scoreHistory.length === 0) {
    return leaderboard.map((row) => ({
      at_seconds: 0,
      player_id: row.player_id,
      player_name: row.display_name,
      score: row.total_points,
    }));
  }

  const nameByPlayerId = new Map(leaderboard.map((row) => [row.player_id, row.display_name]));

  const timeline: TimelinePoint[] = [];
  for (const point of scoreHistory) {
    for (const [playerId, score] of Object.entries(point.scores)) {
      timeline.push({
        at_seconds: point.t,
        player_id: playerId,
        player_name: nameByPlayerId.get(playerId) ?? playerId,
        score,
      });
    }
  }

  return timeline.sort((a, b) => {
    if (a.at_seconds !== b.at_seconds) return a.at_seconds - b.at_seconds;
    return a.player_name.localeCompare(b.player_name);
  });
}
