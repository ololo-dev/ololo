/** Where a session's points came from, at a glance.
 *
 * Shared by the chat's closing card and the session report, so the two can
 * never disagree about what the player scored.
 */
import type {
  PlayerJudgeScoredPayload,
  PlayerProbeEntry,
  PlayerTaskEvaluation,
  PlayerTaskSummaryEntry,
} from "$lib/types/arena";

export type SummaryJudge = {
  slug: string;
  name: string;
  points: number;
  perTask: { ordinal: number; points: number }[];
};

export type SessionSummary = {
  probesRun: number;
  checkPoints: number;
  verdictCount: number;
  judgePoints: number;
  bonusPoints: number;
  tasksPassed: number;
  judges: SummaryJudge[];
  criteria: { key: string; title: string; avg: number }[];
};

/** A task counts as passed once its result says so — the same rule the task
 *  list uses, so the tile and the list cannot disagree. */
export function isTaskPassed(t: PlayerTaskSummaryEntry): boolean {
  return t.result?.status === "completed" || t.result?.status === "correct";
}

export function buildSessionSummary(
  tasks: PlayerTaskSummaryEntry[],
  probesByTask: Map<string, PlayerProbeEntry[]>,
  judgeResultsByTask: Map<string, PlayerJudgeScoredPayload[]>,
  evaluationsByTask: Map<string, PlayerTaskEvaluation>,
): SessionSummary {
  const allProbes = [...probesByTask.values()].flat();
  const allVerdicts = [...judgeResultsByTask.values()].flat();

  const judgeTotals = new Map<
    string,
    { name: string; points: number; perTask: { ordinal: number; points: number }[] }
  >();
  const ordinalByTask = new Map(tasks.map((t) => [t.task_id, t.ordinal]));
  for (const v of allVerdicts) {
    const entry = judgeTotals.get(v.judge_slug) ?? { name: v.judge_name, points: 0, perTask: [] };
    entry.points += v.point_delta;
    entry.perTask.push({ ordinal: ordinalByTask.get(v.task_id) ?? 0, points: v.point_delta });
    judgeTotals.set(v.judge_slug, entry);
  }

  const criteria = new Map<string, { title: string; sum: number; n: number }>();
  for (const ev of evaluationsByTask.values()) {
    for (const c of ev.criteria) {
      for (const s of c.scores ?? []) {
        if (s.score == null) continue;
        const entry = criteria.get(c.key) ?? { title: c.title, sum: 0, n: 0 };
        entry.sum += s.score;
        entry.n += 1;
        criteria.set(c.key, entry);
      }
    }
  }

  return {
    probesRun: allProbes.length,
    checkPoints: allProbes.reduce((sum, p) => sum + p.point_delta, 0),
    verdictCount: allVerdicts.length,
    judgePoints: allVerdicts.reduce((sum, v) => sum + v.point_delta, 0),
    bonusPoints: tasks.reduce((sum, t) => sum + (t.bonus_points ?? 0), 0),
    tasksPassed: tasks.filter(isTaskPassed).length,
    judges: [...judgeTotals.entries()]
      .map(([slug, e]) => ({
        slug,
        ...e,
        perTask: e.perTask.sort((a, b) => a.ordinal - b.ordinal),
      }))
      .sort((a, b) => b.points - a.points),
    criteria: [...criteria.entries()]
      .map(([key, e]) => ({ key, title: e.title, avg: e.sum / e.n }))
      .sort((a, b) => b.avg - a.avg),
  };
}

/** `+7` / `-3` / `0` — a signed chip, so a gain never reads as a loss. */
export function pointsChip(n: number): string {
  return n > 0 ? `+${n}` : `${n}`;
}
