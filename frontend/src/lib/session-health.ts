/**
 * Code-health history on the session and player pages: the per-player
 * payload the snapshot seeds, the live `health_updated` upserts into it,
 * and the derived numbers the UI shows (latest verified score, trend).
 */
import type {
  HealthCheckpointView,
  HealthLevel,
  HealthThresholds,
  SessionHealthPayload,
} from "$lib/types/arena";

/** Participant series colours, shared by every chart and indicator so one
 *  player is the same colour everywhere. Assigned by member index. */
export const SERIES_COLORS = [
  "#6be597",
  "#fb341c",
  "#8fb4ec",
  "#f5a623",
  "#bd10e0",
  "#4a90e2",
  "#7ed321",
];
export function seriesColor(idx: number): string {
  return SERIES_COLORS[idx % SERIES_COLORS.length];
}

/** Colours of the three health levels (dashboard chips + chart bands). */
export const LEVEL_COLORS: Record<HealthLevel, { fg: string; bg: string }> = {
  green: { fg: "#3aa568", bg: "rgba(107,229,151,0.15)" },
  amber: { fg: "#f5a623", bg: "rgba(245,166,35,0.15)" },
  red: { fg: "#fb341c", bg: "rgba(251,52,28,0.15)" },
  unknown: { fg: "#9ea7b6", bg: "rgba(158,167,182,0.15)" },
};

export function levelOf(score: number | null | undefined, t: HealthThresholds): HealthLevel {
  if (score == null || Number.isNaN(score)) return "unknown";
  if (score >= t.green_min) return "green";
  if (score >= t.amber_min) return "amber";
  return "red";
}

/** Insert or replace a checkpoint (by id) in a player's history, keeping
 *  the list ordered by time. Returns a new payload; the input is not
 *  mutated so `$state` sees a fresh reference. */
export function upsertCheckpoint(
  payload: SessionHealthPayload | null,
  playerId: string,
  checkpoint: HealthCheckpointView,
  fallbackThresholds: HealthThresholds = { green_min: 70, amber_min: 55 },
): SessionHealthPayload {
  const base: SessionHealthPayload = payload ?? {
    thresholds: fallbackThresholds,
    players: {},
  };
  const player = base.players[playerId] ?? { checkpoints: [], task_ranges: [] };
  const rest = player.checkpoints.filter((c) => c.id !== checkpoint.id);
  const checkpoints = [...rest, checkpoint].sort(byTime);
  return {
    thresholds: base.thresholds,
    players: {
      ...base.players,
      [playerId]: { ...player, checkpoints },
    },
  };
}

function byTime(a: HealthCheckpointView, b: HealthCheckpointView): number {
  const ta = a.t ?? Date.parse(a.created_at);
  const tb = b.t ?? Date.parse(b.created_at);
  return ta - tb;
}

/** What the indicator next to a participant shows. */
export interface HealthIndicator {
  /** The latest server-verified score, else the latest client score. */
  score: number | null;
  level: HealthLevel;
  /** Direction against the previous checkpoint with a score. */
  trend: "up" | "down" | "flat" | null;
  verified: boolean;
  /** How many checkpoints carry a score at all. */
  points: number;
}

export function indicatorFor(
  payload: SessionHealthPayload | null,
  playerId: string,
): HealthIndicator | null {
  const history = payload?.players[playerId];
  if (!history || history.checkpoints.length === 0) return null;
  const scored = history.checkpoints.filter((c) => c.score != null);
  if (scored.length === 0) {
    return { score: null, level: "unknown", trend: null, verified: false, points: 0 };
  }
  const verified = scored.filter((c) => c.server_status === "ok");
  const line = verified.length > 0 ? verified : scored;
  const last = line[line.length - 1];
  const prev = line.length > 1 ? line[line.length - 2] : null;
  const score = last.score ?? null;
  let trend: HealthIndicator["trend"] = null;
  if (prev && prev.score != null && score != null) {
    const d = score - prev.score;
    trend = Math.abs(d) < 0.05 ? "flat" : d > 0 ? "up" : "down";
  }
  return {
    score,
    level: levelOf(score, payload!.thresholds),
    trend,
    verified: verified.length > 0,
    points: scored.length,
  };
}

/** Short hash for tooltips. */
export function shortSha(sha: string): string {
  return sha.startsWith("none:") ? "—" : sha.slice(0, 7);
}

export function formatScore(score: number | null | undefined): string {
  return score == null ? "—" : score.toFixed(1);
}
