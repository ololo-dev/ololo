import type { PlayerHistoryCommit } from "$lib/types/arena";

export type AttributionResult = {
  baseline: PlayerHistoryCommit | null;
  attributed: Map<string, PlayerHistoryCommit[]>;
  unattributed: PlayerHistoryCommit[];
};

/** The slice of a task the attributor needs: identity plus its work window. */
export type AttributionTask = {
  task_id: string;
  ordinal: number;
  started_at: string | null;
};

const BASELINE_RE = /^ololo snapshot: session start @/;
// Any task-addressed commit belongs to that task's Changes view: the final
// feat() snapshot, wip() checkpoints, and the auxiliary artifact/flag/memory
// commits ololo stamps with the task in play.
const FEAT_RE = /^(?:feat|wip|artifact|flag|memory)\(([\w-]+)\):\s*(.+)/;

export function attributeCommits(
  commits: PlayerHistoryCommit[],
  tasks: AttributionTask[] = [],
): AttributionResult {
  const baseline: PlayerHistoryCommit[] = [];
  const attributed = new Map<string, PlayerHistoryCommit[]>();
  const unattributed: PlayerHistoryCommit[] = [];

  for (const c of commits) {
    const msg = c.message.trim();
    if (BASELINE_RE.test(msg)) {
      baseline.push(c);
      continue;
    }
    const m = FEAT_RE.exec(msg);
    if (m) {
      const id = m[1];
      const arr = attributed.get(id);
      if (arr) arr.push(c);
      else attributed.set(id, [c]);
    } else {
      unattributed.push(c);
    }
  }

  // Commits without a task-addressed message (legacy `artifact: sync`,
  // `flag: …`, memory snapshots, the final snapshot) still belong to a task:
  // the one whose work window contains them. Window = [task.started_at, next
  // task's started_at); the last window is open-ended, and commits that
  // predate every window (clock skew) fold into the first task so nothing
  // disappears from the per-task view.
  // Times come from different clocks (git commits carry the player's local
  // offset, task windows are UTC) — compare epochs, never strings.
  const windows = tasks
    .filter((t) => t.started_at !== null)
    .map((t) => ({ task_id: t.task_id, startMs: Date.parse(t.started_at!) }))
    .filter((t) => Number.isFinite(t.startMs))
    .sort((a, b) => a.startMs - b.startMs);
  if (windows.length > 0) {
    for (const c of unattributed) {
      const atMs = Date.parse(c.author_time);
      let target: (typeof windows)[number] | null = null;
      for (const t of windows) {
        if (Number.isFinite(atMs) && atMs >= t.startMs) target = t;
        else break;
      }
      const taskId = (target ?? windows[0]).task_id;
      const arr = attributed.get(taskId);
      if (arr) arr.push(c);
      else attributed.set(taskId, [c]);
    }
    unattributed.length = 0;
  }

  for (const arr of attributed.values()) {
    arr.sort((a, b) => a.author_time.localeCompare(b.author_time));
  }

  return {
    baseline:
      baseline.length > 0
        ? baseline.sort((a, b) => a.author_time.localeCompare(b.author_time))[0]
        : null,
    attributed,
    unattributed,
  };
}
