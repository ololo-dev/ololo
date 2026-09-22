/**
 * The score chart's data: one line per participant, every probe on it.
 *
 * The x axis is the union of the score history's instants and the health
 * checkpoints' instants. Between score events a participant's line carries
 * its last total, so a probe that changed no points still sits on the line
 * — that is where its health marker goes. Each point remembers what it is
 * (a checkpoint, a scored change, or both) for the markers and the tooltip.
 */
import type {
  HealthCheckpointView,
  ScoreChange,
  ScoreHistoryPoint,
  SessionHealthPayload,
  TaskRangeView,
} from "$lib/types/arena";

export type ChartMember = { id: string; player_id: string; display_name: string };

/** What one point of one participant's line stands for. */
export interface PointMeta {
  t: number;
  /** The participant's total at this instant. */
  total: number;
  /** Points gained or lost here; `null` when nothing was scored (a probe
   *  that only produced a checkpoint, or a carried value). */
  delta: number | null;
  /** The scored rows behind `delta`, when the server said (empty for a live
   *  point the browser appended: the delta is then the totals' difference). */
  changes: ScoreChange[];
  /** The health checkpoint at this instant, when there is one (the last
   *  one when several share the second). */
  checkpoint: HealthCheckpointView | null;
}

export interface ChartData {
  xs: number[];
  /** One series per member, aligned with `xs`; `null` past the reveal cut. */
  series: (number | null)[][];
  /** Keyed by `${memberIndex}:${xIndex}`. Only instants where the member
   *  has a checkpoint or a scored change are present. */
  meta: Map<string, PointMeta>;
}

function memberScore(scores: Record<string, number>, m: ChartMember): number | undefined {
  return scores[m.id] ?? scores[m.player_id];
}

function ownsChange(c: ScoreChange, m: ChartMember): boolean {
  return c.player_id === m.player_id || c.player_id === m.id;
}

export function buildChartData(
  members: ChartMember[],
  scoreHistory: ScoreHistoryPoint[],
  health: SessionHealthPayload | null,
  cut: number | null = null,
): ChartData {
  const times = new Set<number>();
  const sampleAt = new Map<number, ScoreHistoryPoint>();
  for (const pt of scoreHistory) {
    times.add(pt.t);
    sampleAt.set(pt.t, pt);
  }
  // `${memberIndex}:${t}` → the checkpoints at that instant.
  const checkpointsAt = new Map<string, HealthCheckpointView[]>();
  if (health) {
    members.forEach((m, i) => {
      for (const cp of health.players[m.player_id]?.checkpoints ?? []) {
        if (cp.t == null) continue;
        times.add(cp.t);
        const key = `${i}:${cp.t}`;
        const list = checkpointsAt.get(key) ?? [];
        list.push(cp);
        checkpointsAt.set(key, list);
      }
    });
  }
  const xs = [...times].sort((a, b) => a - b);
  const meta = new Map<string, PointMeta>();
  const series = members.map((m, i) => {
    let last = 0;
    return xs.map((t, idx) => {
      const sample = sampleAt.get(t);
      const scored = sample ? memberScore(sample.scores, m) : undefined;
      const total = scored ?? last;
      const cps = checkpointsAt.get(`${i}:${t}`);
      const checkpoint = cps && cps.length > 0 ? cps[cps.length - 1] : null;
      const changes = (sample?.changes ?? []).filter((c) => ownsChange(c, m));
      let delta: number | null = null;
      if (changes.length > 0) {
        delta = changes.reduce((sum, c) => sum + c.delta, 0);
      } else if (scored !== undefined && scored !== last) {
        delta = scored - last;
      }
      last = total;
      if (cut != null && t > cut) return null;
      if (checkpoint || delta !== null) {
        meta.set(`${i}:${idx}`, { t, total, delta, changes, checkpoint });
      }
      return total;
    });
  });
  return { xs, series, meta };
}

/** The task a participant was on at `t`, from the history's task ranges. */
export function taskAt(ranges: TaskRangeView[], t: number): TaskRangeView | null {
  let found: TaskRangeView | null = null;
  for (const r of ranges) {
    if (r.start_t == null || r.start_t > t) continue;
    if (r.end_t != null && r.end_t < t) continue;
    found = r;
  }
  return found;
}

/** `+10 check passed`, `−5 check failed`, `+14 judge · Performance`, … */
export function describeChange(c: ScoreChange): string {
  const amount = `${c.delta > 0 ? "+" : c.delta < 0 ? "−" : ""}${Math.abs(c.delta)}`;
  switch (c.kind) {
    case "probe":
      return `${amount} ${c.delta > 0 ? "check passed" : c.delta < 0 ? "check failed" : "check"}`;
    case "completion_bonus":
      return `${amount} task bonus`;
    case "health_bonus":
      return `${amount} health bonus`;
    case "similarity_penalty":
      return `${amount} similarity penalty`;
    case "judge":
      return `${amount} judge${c.label ? ` · ${c.label}` : ""}`;
    default:
      return `${amount} ${c.kind.replace(/_/g, " ")}`;
  }
}

/** The points line of a tooltip: the total, then what moved it. */
export function describePoints(meta: PointMeta): string {
  if (meta.delta === null) return String(meta.total);
  if (meta.changes.length > 0) {
    return `${meta.total} (${meta.changes.map(describeChange).join(", ")})`;
  }
  const sign = meta.delta > 0 ? "+" : "−";
  return `${meta.total} (${sign}${Math.abs(meta.delta)})`;
}
