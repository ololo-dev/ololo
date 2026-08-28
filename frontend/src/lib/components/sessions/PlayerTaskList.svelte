<script lang="ts">
  import PlayerTaskPanel from "./PlayerTaskPanel.svelte";
  import type {
    PlayerTaskSummaryEntry,
    PlayerProbeEntry,
    PlayerHistoryCommit,
    PlayerJudgeScoredPayload,
    PlayerJudgeStatusPayload,
    PlayerTaskEvaluation,
    TaskStatsEntry,
  } from "$lib/types/arena";

  type TaskSortKey = "date" | "status";

  let {
    sortedTasks,
    taskSort,
    cycleSort,
    probesByTask,
    changesByTask,
    judgeResultsByTask = new Map(),
    judgeStatusesByTask = new Map(),
    evaluationsByTask = new Map(),
    taskStatsByOrdinal = new Map(),
    isTaskPassed,
    taskTypeProgress,
  }: {
    sortedTasks: PlayerTaskSummaryEntry[];
    taskSort: { key: TaskSortKey; dir: "asc" | "desc" };
    cycleSort: (key: TaskSortKey) => void;
    probesByTask: Map<string, PlayerProbeEntry[]>;
    changesByTask: Map<
      string,
      { commits: PlayerHistoryCommit[]; mode: "range" | "per-commit" }
    >;
    judgeResultsByTask?: Map<string, PlayerJudgeScoredPayload[]>;
    judgeStatusesByTask?: Map<string, PlayerJudgeStatusPayload[]>;
    evaluationsByTask?: Map<string, PlayerTaskEvaluation>;
    taskStatsByOrdinal?: Map<number, TaskStatsEntry>;
    isTaskPassed: (taskId: string) => boolean;
    taskTypeProgress: (taskId: string) => { passed: number; total: number };
  } = $props();
</script>

<div>
  {#if sortedTasks.length === 0}
    <div class="overflow-hidden rounded-[8px] bg-white shadow-sm">
      <div
        class="flex flex-col items-center justify-center py-16 text-brand-muted"
      >
        <svg
          width="40"
          height="40"
          viewBox="0 0 24 24"
          fill="none"
          class="mb-3 opacity-40"
          aria-hidden="true"
        >
          <rect
            x="3"
            y="3"
            width="18"
            height="18"
            rx="2"
            stroke="currentColor"
            stroke-width="2"
          />
          <path
            d="M9 12h6M9 8h6M9 16h4"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
          />
        </svg>
        <p class="text-sm">No tasks yet.</p>
      </div>
    </div>
  {:else}
    <!-- Sort controls -->
    <div class="mb-3 flex items-center gap-2">
      <span class="text-[12px] font-semibold text-brand-muted">Sort by:</span>
      <button
        type="button"
        onclick={() => cycleSort("date")}
        class="rounded-md px-2.5 py-1 text-[12px] font-semibold transition-colors {taskSort.key ===
        'date'
          ? 'bg-brand-blue/10 text-brand-blue'
          : 'text-brand-muted hover:text-brand-text'}"
        >Date{taskSort.key === "date"
          ? taskSort.dir === "desc"
            ? " ↓"
            : " ↑"
          : ""}</button
      >
      <button
        type="button"
        onclick={() => cycleSort("status")}
        class="rounded-md px-2.5 py-1 text-[12px] font-semibold transition-colors {taskSort.key ===
        'status'
          ? 'bg-brand-blue/10 text-brand-blue'
          : 'text-brand-muted hover:text-brand-text'}"
        >Status{taskSort.key === "status"
          ? taskSort.dir === "asc"
            ? " ↑"
            : " ↓"
          : ""}</button
      >
    </div>

    <div class="space-y-3">
      {#each sortedTasks as task (task.task_id)}
        {@const progress = taskTypeProgress(task.task_id)}
        {@const changes = changesByTask.get(task.task_id) ?? {
          commits: [],
          mode: "per-commit" as const,
        }}
        {@const statusBadge =
          task.scheduler_state?.state === "judging"
            ? "judging"
            : (task.result?.status ?? "pending")}
        <div class="overflow-hidden rounded-[8px] bg-white shadow-sm">
          <PlayerTaskPanel
            {task}
            probes={probesByTask.get(task.task_id) ?? []}
            {statusBadge}
            passedTypes={progress.passed}
            totalTypes={progress.total}
            {changes}
            judgeResults={judgeResultsByTask.get(task.task_id) ?? []}
            judgeStatuses={judgeStatusesByTask.get(task.task_id) ?? []}
            evaluation={evaluationsByTask.get(task.task_id) ?? null}
            stats={taskStatsByOrdinal.get(task.ordinal) ?? null}
            defaultExpanded={!isTaskPassed(task.task_id)}
          />
        </div>
      {/each}
    </div>
  {/if}
</div>
