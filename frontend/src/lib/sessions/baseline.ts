import type { PlayerHistoryCommit, PlayerTaskSummaryEntry } from "$lib/types/arena";

export type BaselineResult = {
  parentCommit: PlayerHistoryCommit | null;
  commits: PlayerHistoryCommit[];
  mode: "range" | "per-commit";
};

export function resolveBaseline(
  taskOrdinal: number,
  tasks: PlayerTaskSummaryEntry[],
  attributed: Map<string, PlayerHistoryCommit[]>,
  baseline: PlayerHistoryCommit | null,
): BaselineResult {
  const sortedTasks = [...tasks].sort((a, b) => a.ordinal - b.ordinal);
  const currentTask = sortedTasks.find((t) => t.ordinal === taskOrdinal);

  const commits = currentTask
    ? [...(attributed.get(currentTask.task_id) ?? [])].sort((a, b) =>
        a.author_time.localeCompare(b.author_time),
      )
    : [];

  let parentCommit: PlayerHistoryCommit | null = null;

  for (let ord = taskOrdinal - 1; ord >= 0; ord--) {
    const prev = sortedTasks.find((t) => t.ordinal === ord);
    if (!prev) continue;
    const prevCommits = (attributed.get(prev.task_id) ?? []).sort((a, b) =>
      a.author_time.localeCompare(b.author_time),
    );
    if (prevCommits.length > 0) {
      parentCommit = prevCommits[prevCommits.length - 1];
      break;
    }
  }

  // The session-start snapshot is the diff base for the first task, never a
  // change of its own — showing it would dump the whole seed project into
  // Task 0's Changes tab.
  if (!parentCommit && baseline) {
    parentCommit = baseline;
  }

  const mode: "range" | "per-commit" = parentCommit && commits.length > 0 ? "range" : "per-commit";

  return { parentCommit, commits, mode };
}
