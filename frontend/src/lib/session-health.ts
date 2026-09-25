/**
 * Code-health history on the session and player pages: the per-player
 * payload the snapshot seeds, the live `health_updated` upserts into it,
 * and the derived numbers the UI shows (latest verified score, trend).
 */
import type {
  HealthCheckpointView,
  HealthLevel,
  HealthTestsView,
  HealthThresholds,
  PlayerTestCommandsView,
  SessionHealthPayload,
  SuiteResult,
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

/** jscpd's letter for a score, when neither side reported one. */
export function gradeFromScore(score: number | null | undefined): string | null {
  if (score == null || Number.isNaN(score)) return null;
  if (score >= 85) return "A";
  if (score >= 70) return "B";
  if (score >= 55) return "C";
  if (score >= 40) return "D";
  return "E";
}

/** The grade shown for a checkpoint: the score's own (the tests composed
 *  in), else the server's once verified, the client's until then, derived
 *  from the score when neither says. */
export function gradeOf(cp: HealthCheckpointView): string | null {
  if (cp.score == null) return null;
  if (cp.grade) return cp.grade;
  const side = cp.server_status === "ok" ? cp.server : (cp.client ?? cp.server);
  return side?.grade ?? cp.server?.grade ?? cp.client?.grade ?? gradeFromScore(cp.score);
}

/** What the indicator next to a participant shows. */
export interface HealthIndicator {
  /** The latest server-verified score, else the latest client score. */
  score: number | null;
  /** Its letter grade (A–E), `null` without a score. */
  grade: string | null;
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
    return { score: null, grade: null, level: "unknown", trend: null, verified: false, points: 0 };
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
    grade: gradeOf(last),
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

/** A tooltip row: what, the value, and the level colour to paint it in. */
export interface TooltipLine {
  label: string;
  value: string;
  tone?: HealthLevel;
}

/** What a run of the tests said, in words: the counts when the output had
 *  them, else the exit code. */
export function testsVerdict(result: SuiteResult): string {
  const c = result.counts;
  if (c && c.passed + c.failed > 0) {
    const parts = [`${c.passed} passed`];
    if (c.failed > 0) parts.push(`${c.failed} failed`);
    if (c.skipped) parts.push(`${c.skipped} skipped`);
    return parts.join(" · ");
  }
  if (c) return "no tests ran";
  if (result.exit_code === 0) return "passed (exit 0)";
  return result.exit_code == null ? "killed" : `failed (exit ${result.exit_code})`;
}

function failing(result: SuiteResult): boolean {
  const c = result.counts;
  if (c) return c.failed > 0;
  return result.exit_code !== 0;
}

/** The project's tests in a checkpoint's tooltip: what the counted run
 *  said and scored, where its log is, and a check's own run that ended
 *  without numbers. Without a run, what the player's docs and manifests
 *  gave (`commands`, when the session scores tests): no test command at
 *  all, or one that has not run yet — so a chart without tests says why.
 *  A run without coverage says why too: its coverage command printed no
 *  summary, the docs name none, or one was named after that run. */
export function testsLines(
  tests: HealthTestsView | null | undefined,
  commands?: PlayerTestCommandsView | null,
): TooltipLine[] {
  if (!tests) {
    if (!commands) return [];
    const command = commands.coverage ?? commands.test;
    return [
      command
        ? { label: "tests", value: `not run yet · ${command}` }
        : { label: "tests", value: "no test command in the docs or manifests", tone: "amber" },
    ];
  }
  const lines: TooltipLine[] = [];
  const run = tests.counted;
  if (run) {
    let value = testsVerdict(run.result);
    if (tests.tests_score != null) value += ` · score ${Math.round(tests.tests_score)}`;
    if (run.inherited) value += ` · from check #${run.probe_seq}`;
    lines.push({ label: "tests", value, tone: failing(run.result) ? "red" : undefined });
    const pct = run.result.coverage_pct;
    if (pct != null) {
      const score =
        tests.coverage_score != null ? ` · score ${Math.round(tests.coverage_score)}` : "";
      lines.push({ label: "coverage", value: `${pct.toFixed(1)}%${score}` });
    } else if (run.coverage_run) {
      lines.push({
        label: "coverage",
        value: "not measured · the coverage run printed no summary",
        tone: "amber",
      });
    } else if (commands && !commands.coverage) {
      lines.push({
        label: "coverage",
        value: "not measured · no coverage command in the docs or manifests",
        tone: "amber",
      });
    } else if (commands?.coverage) {
      lines.push({ label: "coverage", value: `not measured yet · ${commands.coverage}` });
    }
    if (run.log) lines.push({ label: "log", value: run.log });
  }
  const attempt = tests.attempt;
  if (attempt) {
    const what =
      attempt.status === "timeout"
        ? "this check's run timed out"
        : attempt.status === "declined"
          ? `not allowed by .ololo/settings.json (${attempt.command})`
          : `could not start${attempt.error ? `: ${attempt.error}` : ""}`;
    lines.push({ label: run ? "this run" : "tests", value: what.slice(0, 120), tone: "amber" });
  }
  return lines;
}
