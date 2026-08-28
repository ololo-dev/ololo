<script lang="ts">
  import { untrack } from 'svelte';
  import type { PlayerTaskSummaryEntry, PlayerProbeEntry, PlayerHistoryCommit, PlayerJudgeScoredPayload, PlayerJudgeStatusPayload, PlayerTaskEvaluation, TaskStatsEntry } from '$lib/types/arena';
  import PlayerTodoChecklist from './PlayerTodoChecklist.svelte';
  import { formatTokens } from '$lib/format';
  import { probeBriefing } from '$lib/sessions/probe-briefing';
  import CopyForAgentButton from '$lib/components/CopyForAgentButton.svelte';
  import TaskChanges from './TaskChanges.svelte';

  type Props = {
    task: PlayerTaskSummaryEntry;
    probes: PlayerProbeEntry[];
    statusBadge: string;
    passedTypes: number;
    totalTypes: number;
    changes: { commits: PlayerHistoryCommit[]; mode: 'range' | 'per-commit' };
    defaultExpanded: boolean;
    judgeResults?: PlayerJudgeScoredPayload[];
    judgeStatuses?: PlayerJudgeStatusPayload[];
    stats?: TaskStatsEntry | null;
    /** This task's open-ended evaluation state (criteria, TODO), when any. */
    evaluation?: PlayerTaskEvaluation | null;
  };

  let { task, probes, statusBadge, passedTypes, totalTypes, changes, defaultExpanded, judgeResults = [], judgeStatuses = [], stats = null, evaluation = null }: Props = $props();

  // One row per attached judge: the lifecycle entry (pending/running/failed)
  // joined with its verdict once scored. Sessions predating judge statuses
  // only have verdicts — synthesize scored rows from them.
  type JudgeRowView = {
    slug: string;
    name: string;
    status: string;
    error?: string | null;
    updated_at?: string | null;
    result?: PlayerJudgeScoredPayload;
  };
  const judgeRows = $derived.by<JudgeRowView[]>(() => {
    const resultBySlug = new Map(judgeResults.map((r) => [r.judge_slug, r]));
    if (judgeStatuses.length === 0) {
      return judgeResults.map((r) => ({
        slug: r.judge_slug,
        name: r.judge_name,
        status: 'scored',
        result: r,
      }));
    }
    return judgeStatuses.map((s) => ({
      slug: s.judge_slug,
      name: s.judge_name,
      status: s.status,
      error: s.error,
      updated_at: s.updated_at,
      result: s.status === 'scored' ? resultBySlug.get(s.judge_slug) : undefined,
    }));
  });

  type TabId = 'probes' | 'todo' | 'changes' | 'judges' | 'stats';
  let tab = $state<TabId>('probes');
  let open = $state(untrack(() => defaultExpanded));
  let openProbes = $state(new Set<string>());

  // Data-dependent tabs are only shown once their data exists. Visibility is
  // derived from live props (WS judge frames, history/stats refetches), so
  // tabs appear in realtime as information arrives.
  const hasChanges = $derived(changes.commits.length > 0);
  // An open-ended task earns the tab even before its first judge row lands:
  // the evaluation contract is the thing being judged.
  const hasJudges = $derived(judgeRows.length > 0 || evaluation != null);
  const hasStats = $derived(!!stats && stats.agents.length > 0);
  // Flag-file completed tasks (no TODO checklist) never earn the tab.
  const hasTodo = $derived((evaluation?.todo?.total ?? 0) > 0);
  const visibleTabs = $derived.by(() => {
    const t: TabId[] = ['probes'];
    if (hasTodo) t.push('todo');
    if (hasChanges) t.push('changes');
    if (hasJudges) t.push('judges');
    if (hasStats) t.push('stats');
    return t;
  });

  // If the active tab's data disappears (e.g. refetch returns empty), fall
  // back to Probes rather than showing a panel without its tab.
  $effect(() => {
    if (!visibleTabs.includes(tab)) {
      tab = 'probes';
    }
  });

  // Header change pulses: bump a counter when live data changes so the
  // corresponding header element re-mounts (via {#key}) and replays its
  // CSS pulse animation. First run only records the baseline — no pulse
  // on initial render.
  let pointsPulse = $state(0);
  let probesPulse = $state(0);
  let judgesPulse = $state(0);
  let statsPulse = $state(0);
  let prevPoints: string | null = null;
  let prevProbes: number | null = null;
  let prevJudges: number | null = null;
  let prevStats: number | null = null;
  $effect(() => {
    const v = `${task.total_points ?? 0}:${task.bonus_points ?? 0}`;
    if (prevPoints !== null && v !== prevPoints) pointsPulse++;
    prevPoints = v;
  });
  $effect(() => {
    const v = probes.length;
    if (prevProbes !== null && v !== prevProbes) probesPulse++;
    prevProbes = v;
  });
  $effect(() => {
    const v = judgeRows.filter((r) => r.status !== 'pending').length;
    if (prevJudges !== null && v !== prevJudges) judgesPulse++;
    prevJudges = v;
  });
  $effect(() => {
    const v = stats?.agents.length ?? 0;
    if (prevStats !== null && v !== prevStats) statsPulse++;
    prevStats = v;
  });

  // Collapse when task transitions to passed via WS.
  $effect(() => {
    if (statusBadge === 'completed' || statusBadge === 'passed' || statusBadge === 'correct') {
      open = false;
    }
  });

  function isProbeOpen(id: string): boolean { return openProbes.has(id); }
  function toggleProbeOpen(id: string): void {
    const next = new Set(openProbes);
    if (next.has(id)) next.delete(id); else next.add(id);
    openProbes = next;
  }

  const sortedProbes = $derived(
    [...probes].sort((a, b) => {
      const ta = a.dispatched_at ? new Date(a.dispatched_at).getTime() : 0;
      const tb = b.dispatched_at ? new Date(b.dispatched_at).getTime() : 0;
      return tb - ta;
    }),
  );

  // Auto-expand the latest probe (first in sortedProbes) when probes change.
  // Collapse all previous probes when a new failed probe arrives.
  // Don't re-expand if user manually collapsed it.
  let autoExpanded = $state<string | null>(null);
  $effect(() => {
    if (sortedProbes.length === 0) return;
    const latest = sortedProbes[0];
    // Only auto-expand when the latest probe changes (new probe arrived).
    if (latest.id !== autoExpanded) {
      autoExpanded = latest.id;
      const isFail = latest.outcome === 'error' || latest.outcome === 'no_response'
        || latest.result?.status === 'failed' || latest.result?.status === 'incorrect';
      if (isFail) {
        // New failed probe: collapse all, only expand the new one
        openProbes = new Set([latest.id]);
      } else if (!openProbes.has(latest.id)) {
        const next = new Set(openProbes);
        next.add(latest.id);
        openProbes = next;
      }
    }
  });

  function statusBadgeClass(status: string): string {
    switch (status) {
      case 'completed':
      case 'passed':
      case 'correct':   return 'bg-green-100 text-green-700';
      case 'failed':
      case 'error':
      case 'incorrect': return 'bg-red-100 text-red-600';
      case 'pending':   return 'bg-amber-100 text-amber-700';
      case 'judging':   return 'bg-blue-100 text-blue-700';
      default:          return 'bg-gray-100 text-gray-500';
    }
  }

  function probeBadgeClass(probe: PlayerProbeEntry): string {
    if (probe.result) return statusBadgeClass(probe.result.status);
    return 'bg-amber-100 text-amber-700';
  }

  function probeBadgeText(probe: PlayerProbeEntry): string {
    if (probe.result) {
      const s = probe.result.status;
      switch (s) {
        case 'passed': case 'correct': return 'Passed';
        case 'failed': case 'error': case 'incorrect': return 'Failed';
        case 'pending': return 'Pending';
        default: return s;
      }
    }
    return 'pending';
  }

  const statsWindowSecs = $derived.by(() => {
    if (!stats?.window_started_at || !stats?.window_ended_at) return null;
    const ms = new Date(stats.window_ended_at).getTime() - new Date(stats.window_started_at).getTime();
    return ms >= 0 ? Math.round(ms / 1000) : null;
  });

  function sortedCounts(rec: Record<string, number>): [string, number][] {
    return Object.entries(rec).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  }

  /**
   * Label for a probe row: which test type it exercised, and the attempt
   * number *only* once there has been more than one.
   *
   * A retry is dispatched as a new probe rather than by incrementing this,
   * so in practice nearly every row is attempt 1 and printing it was noise.
   * Built as a string rather than inline markup so the spacing cannot depend
   * on how the template happens to collapse whitespace.
   */
  function probeLabel(probe: PlayerProbeEntry): string {
    const attempt = probe.attempt > 1 ? ` · attempt ${probe.attempt}` : '';
    return probe.test_ordinal != null
      ? probe.test_ordinal >= 2000
        ? `Judge probe${attempt}`
        : `Probe type ${probe.test_ordinal + 1}${attempt}`
      : `Probe${probe.attempt > 1 ? ` #${probe.attempt}` : ''}`;
  }

  /**
   * A header badge names a section, so clicking it should take you there:
   * reveal the body and select that tab. Expanding rather than toggling is
   * deliberate — clicking "judges" on an open panel should show judges, not
   * collapse the panel out from under the click.
   */
  function openSection(id: TabId) {
    tab = id;
    open = true;
  }

  function handleKeydown(e: KeyboardEvent) {
    const tabs = visibleTabs;
    const idx = tabs.indexOf(tab);
    if (e.key === 'ArrowRight') {
      e.preventDefault();
      tab = tabs[(idx + 1) % tabs.length];
    } else if (e.key === 'ArrowLeft') {
      e.preventDefault();
      tab = tabs[(idx - 1 + tabs.length) % tabs.length];
    }
  }
</script>

<div data-testid="task-panel-{task.ordinal}" class="overflow-hidden rounded-[8px] bg-white shadow-sm">
  <!-- The header is split into three: a toggle on the left, the section
       badges (real buttons — they jump to their tab), and a second toggle on
       the right. The badges cannot live inside the toggle because a button
       may not contain another button, and they must not merely bubble into
       the toggle either, or clicking one on an open panel would close it. -->
  <div
    class="flex w-full items-center gap-3 px-6 py-3 transition-colors hover:bg-brand-light-blue/20"
  >
    <button
      type="button"
      onclick={() => (open = !open)}
      aria-expanded={open}
      class="flex min-w-0 flex-1 items-center gap-3 text-left cursor-pointer"
    >
      <!-- The judging explainer is a tooltip, not row content: inlined it
           spanned two lines and squeezed the task title out (audit UI-M8).
           The ⓘ glyph marks that the badge has more to say on hover. -->
      {#if statusBadge === 'judging'}
        <span
          class="inline-flex cursor-help items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-semibold {statusBadgeClass(statusBadge)}"
          data-testid="judging-note-{task.ordinal}"
          title="All probes done — the judge panel is reviewing. A judge may still ask for an artifact; it appears as a new probe."
        >
          {statusBadge}
          <svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/></svg>
        </span>
      {:else}
        <span class="inline-flex items-center rounded-full px-2 py-0.5 text-[10px] font-semibold {statusBadgeClass(statusBadge)}">{statusBadge}</span>
      {/if}
      <span class="font-mono text-[10px] text-brand-blue/70">#{task.ordinal}</span>
      <span class="truncate text-[13px] font-semibold text-brand-text">{task.title}</span>
    </button>

    <!-- Available sections at a glance (mirrors the tab strip) -->
    <span data-testid="task-sections-{task.ordinal}" class="flex shrink-0 items-center gap-1.5">
      {#if evaluation?.todo && evaluation.todo.total > 0}
        <!-- The plan's pulse, visible without expanding the task; a click
             opens the TODO sub-tab like every other section chip. -->
        <button
          type="button"
          data-testid="todo-badge-{task.ordinal}"
          onclick={() => openSection('todo')}
          class="cursor-pointer rounded-full px-2 py-0.5 text-[10px] font-semibold tabular-nums transition-opacity hover:opacity-70 {evaluation.todo.checked >= evaluation.todo.total
            ? 'bg-green-100 text-green-700'
            : 'bg-brand-blue/10 text-brand-blue'}"
        >TODO {evaluation.todo.checked}/{evaluation.todo.total}</button>
      {/if}
      {#key probesPulse}
        <button
          type="button"
          data-testid="task-section-probes-{task.ordinal}"
          onclick={() => openSection('probes')}
          class="cursor-pointer rounded-full bg-brand-blue/10 px-2 py-0.5 text-[10px] font-semibold text-brand-blue transition-opacity hover:opacity-70 {probesPulse > 0 ? 'chip-pulse' : ''}"
        >probes ({probes.length})</button>
      {/key}
      {#if hasChanges}
        <button
          type="button"
          data-testid="task-section-changes-{task.ordinal}"
          onclick={() => openSection('changes')}
          class="cursor-pointer rounded-full bg-brand-blue/10 px-2 py-0.5 text-[10px] font-semibold text-brand-blue transition-opacity hover:opacity-70"
        >changes</button>
      {/if}
      {#if hasJudges}
        {#key judgesPulse}
          <button
            type="button"
            data-testid="task-section-judges-{task.ordinal}"
            onclick={() => openSection('judges')}
            class="cursor-pointer rounded-full px-2 py-0.5 text-[10px] font-semibold transition-opacity hover:opacity-70 {judgeRows.some((r) => r.status === 'failed') ? 'bg-red-100 text-red-600' : judgeRows.some((r) => r.status === 'running') ? 'bg-amber-100 text-amber-700' : 'bg-brand-blue/10 text-brand-blue'} {judgesPulse > 0 ? 'chip-pulse' : ''}"
          >judges{#if judgeRows.some((r) => r.status === 'running')}&nbsp;⏳{/if} ({judgeRows.filter((r) => r.status === 'scored').length}/{judgeRows.length})</button>
        {/key}
      {/if}
      {#if hasStats}
        {#key statsPulse}
          <button
            type="button"
            data-testid="task-section-stats-{task.ordinal}"
            onclick={() => openSection('stats')}
            class="cursor-pointer rounded-full bg-brand-blue/10 px-2 py-0.5 text-[10px] font-semibold text-brand-blue transition-opacity hover:opacity-70 {statsPulse > 0 ? 'chip-pulse' : ''}"
          >stats</button>
        {/key}
      {/if}
    </span>

    <!-- Right-aligned header group; still toggles the panel. -->
    <button
      type="button"
      onclick={() => (open = !open)}
      aria-label={open ? 'Collapse task details' : 'Expand task details'}
      aria-expanded={open}
      class="ml-auto flex shrink-0 cursor-pointer items-center gap-3"
    >
      {#if totalTypes > 0}
        <span class="text-[10px] text-brand-muted">{passedTypes}/{totalTypes} types passed</span>
      {/if}
      {#if task.bonus_points}
        {#key pointsPulse}
          <span
            data-testid="task-bonus-points-{task.ordinal}"
            class="rounded-full bg-amber-100 px-2 py-0.5 text-[10px] font-semibold tabular-nums text-amber-700 {pointsPulse > 0 ? 'chip-pulse' : ''}"
          >bonus {task.bonus_points > 0 ? '+' : ''}{task.bonus_points}</span>
        {/key}
      {/if}
      {#if task.total_points != null}
        {#key pointsPulse}
          <span
            data-testid="task-total-points-{task.ordinal}"
            class="inline-block text-[11px] font-semibold tabular-nums {task.total_points >= 0 ? 'text-green-600' : 'text-red-600'} {pointsPulse > 0 ? 'points-pulse' : ''}"
          >{task.total_points >= 0 ? '+' : ''}{task.total_points} pts</span>
        {/key}
      {/if}
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" aria-hidden="true" class="shrink-0 text-brand-muted transition-transform {open ? 'rotate-180' : ''}">
        <path d="M6 9l6 6 6-6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
      </svg>
    </button>
  </div>

  {#if open}
    <div class="border-t border-brand-border/40 px-6 py-4">
    <!-- Tab strip -->
    <div class="mb-4 flex gap-1 border-b border-brand-border/40" role="tablist" tabindex="-1" onkeydown={handleKeydown}>
      <button
        type="button"
        data-testid="tab-probes"
        role="tab"
        aria-selected={tab === 'probes'}
        onclick={() => (tab = 'probes')}
        class="px-3 py-1.5 text-[12px] font-semibold transition-colors {tab === 'probes' ? 'text-brand-blue border-b-2 border-brand-blue' : 'text-brand-muted hover:text-brand-text'}"
      >Probes{#if probes.length > 0}&nbsp;({probes.length}){/if}</button>
      {#if hasTodo}
        <button
          type="button"
          data-testid="tab-todo"
          role="tab"
          aria-selected={tab === 'todo'}
          onclick={() => (tab = 'todo')}
          class="px-3 py-1.5 text-[12px] font-semibold transition-colors {tab === 'todo' ? 'text-brand-blue border-b-2 border-brand-blue' : 'text-brand-muted hover:text-brand-text'}"
        >TODO{#if evaluation?.todo}&nbsp;({evaluation.todo.checked}/{evaluation.todo.total}){/if}</button>
      {/if}
      {#if hasChanges}
        <button
          type="button"
          data-testid="tab-changes"
          role="tab"
          aria-selected={tab === 'changes'}
          onclick={() => (tab = 'changes')}
          class="px-3 py-1.5 text-[12px] font-semibold transition-colors {tab === 'changes' ? 'text-brand-blue border-b-2 border-brand-blue' : 'text-brand-muted hover:text-brand-text'}"
        >Changes</button>
      {/if}
      {#if hasJudges}
        <button
          type="button"
          data-testid="tab-judges"
          role="tab"
          aria-selected={tab === 'judges'}
          onclick={() => (tab = 'judges')}
          class="px-3 py-1.5 text-[12px] font-semibold transition-colors {tab === 'judges' ? 'text-brand-blue border-b-2 border-brand-blue' : 'text-brand-muted hover:text-brand-text'}"
        >Judges&nbsp;({judgeRows.filter((r) => r.status === 'scored').length}/{judgeRows.length})</button>
      {/if}
      {#if hasStats}
        <button
          type="button"
          data-testid="tab-stats"
          role="tab"
          aria-selected={tab === 'stats'}
          onclick={() => (tab = 'stats')}
          class="px-3 py-1.5 text-[12px] font-semibold transition-colors {tab === 'stats' ? 'text-brand-blue border-b-2 border-brand-blue' : 'text-brand-muted hover:text-brand-text'}"
        >Stats&nbsp;({stats?.agents.length ?? 0})</button>
      {/if}
    </div>

    {#if tab === 'probes'}
      <div role="tabpanel" class="space-y-2">
        {#each sortedProbes as probe (probe.id)}
          {@const probeOpen = isProbeOpen(probe.id)}
          <div data-testid="probe-row-{probe.id}" class="rounded-md border border-brand-border/40 overflow-hidden">
            <button
              type="button"
              onclick={() => toggleProbeOpen(probe.id)}
              class="flex w-full items-center gap-3 px-4 py-2.5 text-left cursor-pointer hover:bg-brand-light-blue/20"
            >
              <span class="inline-flex items-center rounded-full px-2 py-0.5 text-[10px] font-semibold {probeBadgeClass(probe)}">{probeBadgeText(probe)}</span>
              <!-- The attempt number is only worth showing once there has
                   been more than one: a probe is retried by dispatching a new
                   one, so in practice almost every row is attempt 1 and the
                   label was pure noise. -->
              <span class="font-mono text-[10px] text-brand-muted">{probeLabel(probe)}</span>
              <span class="text-[11px] font-semibold" style="color: {probe.point_delta > 0 ? '#22c55e' : probe.point_delta < 0 ? '#ef4444' : '#6b7280'};">{probe.point_delta > 0 ? '+' : ''}{probe.point_delta} pts</span>
              {#if probe.dispatched_at}
                <span class="ml-auto text-[10px] text-brand-muted">{new Date(probe.dispatched_at).toLocaleString()}</span>
              {/if}
              <svg width="10" height="10" viewBox="0 0 24 24" fill="none" aria-hidden="true" class="shrink-0 text-brand-muted transition-transform {probeOpen ? 'rotate-180' : ''}"><path d="M6 9l6 6 6-6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>
            </button>
            {#if probeOpen}
              <div class="border-t border-brand-border/40 bg-brand-light-blue/10 px-4 py-3 space-y-3 text-[12px]">
                <!-- This probe and its task as one paste for a coding agent —
                     the same briefing the slides view hands over, so a player
                     gets it from whichever view they happen to be in. -->
                <div class="flex justify-end">
                  <CopyForAgentButton
                    compact
                    testid="copy-briefing-{probe.id}"
                    text={probeBriefing(task, probe, probeBadgeText(probe))}
                  />
                </div>
                {#if task.content}
                  <div>
                    <div class="mb-1 text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Task Description</div>
                    <div class="text-brand-text whitespace-pre-wrap">{task.content}</div>
                  </div>
                {/if}
                {#if probe.rendered_command}
                  <div>
                    <div class="mb-1 text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Command</div>
                    <pre class="overflow-x-auto rounded-md bg-gray-900 px-3 py-2 text-[11px] text-green-400 whitespace-pre-wrap break-all"><code>{probe.rendered_command}</code></pre>
                  </div>
                {/if}
                {#if probe.fixture_values}
                  <div>
                    <div class="mb-1 text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Fixtures</div>
                    <pre class="overflow-x-auto rounded-md bg-white px-3 py-2 text-[11px] text-brand-text/80 whitespace-pre-wrap break-all border border-brand-border/40">{probe.fixture_values}</pre>
                  </div>
                {/if}
                {#if probe.expected_answer || probe.result?.expected != null}
                  <div>
                    <div class="mb-1 text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Expected Answers</div>
                    <div class="rounded-md border border-brand-border/60 bg-white divide-y divide-brand-border/40">
                      {#if probe.expected_answer}
                        <div class="px-3 py-2"><span class="text-brand-muted">Task answer: </span><code class="rounded bg-gray-100 px-1 text-brand-text">{probe.expected_answer}</code></div>
                      {/if}
                      {#if probe.result?.expected != null}
                        <div class="px-3 py-2"><span class="text-brand-muted">Expected: </span><code class="rounded bg-green-50 px-1 text-green-700">{probe.result.expected}</code></div>
                      {/if}
                      {#if probe.result?.actual != null}
                        <div class="px-3 py-2"><span class="text-brand-muted">Actual: </span><code class="rounded bg-red-50 px-1 text-red-700">{probe.result.actual}</code></div>
                      {/if}
                    </div>
                  </div>
                {:else if probe.result?.actual != null}
                  <!-- Expected value is secret for this probe type: show the
                       player's response with a note instead of a bare
                       "Actual:" row under an "Expected Answers" heading. -->
                  <div>
                    <div class="mb-1 text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Response</div>
                    <div class="rounded-md border border-brand-border/60 bg-white">
                      <div class="px-3 py-2">
                        <code class="rounded px-1 {probeBadgeText(probe) === 'Passed' ? 'bg-green-50 text-green-700' : 'bg-red-50 text-red-700'}">{probe.result.actual}</code>
                        <span class="ml-2 text-[10px] text-brand-muted">expected value is hidden for this question</span>
                      </div>
                    </div>
                  </div>
                {/if}
                {#if probe.outcome || probe.exit_code !== null || probe.duration_ms !== null}
                  <div class="flex gap-6">
                    {#if probe.outcome}
                      <div><div class="mb-0.5 text-[10px] font-semibold uppercase tracking-wider text-brand-muted">stdout</div><pre class="overflow-x-auto text-[11px] text-brand-text/80 whitespace-pre-wrap break-all">{probe.outcome}</pre></div>
                    {/if}
                    {#if probe.exit_code !== null}
                      <div><div class="mb-0.5 text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Exit</div><code class="font-mono font-semibold {probe.exit_code === 0 ? 'text-green-600' : 'text-red-600'}">{probe.exit_code}</code></div>
                    {/if}
                    {#if probe.duration_ms !== null}
                      <div><div class="mb-0.5 text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Duration</div><code class="font-mono text-brand-text">{probe.duration_ms}ms</code></div>
                    {/if}
                  </div>
                {/if}
                {#if probe.resolved_at}
                  <div class="text-[10px] text-brand-muted">Resolved: {new Date(probe.resolved_at).toLocaleString()}</div>
                {/if}
              </div>
            {/if}
          </div>
        {/each}
      </div>
    {:else if tab === 'changes'}
      <div role="tabpanel">
        <TaskChanges commits={changes.commits} mode={changes.mode} testid="task-changes-{task.ordinal}" />
      </div>
    {:else if tab === 'todo'}
      <div role="tabpanel" data-testid="task-evaluation-{task.ordinal}">
        {#if evaluation}
          <PlayerTodoChecklist {evaluation} />
        {/if}
      </div>
    {:else if tab === 'judges'}
      <div role="tabpanel" data-testid="task-judges-{task.ordinal}">
        {#if evaluation}
          <!-- The task's own evaluation state: several open-ended tasks can
               coexist in one project, so this renders per task, not globally. -->

        {/if}
        {#if judgeRows.length === 0}
          <p class="py-6 text-center text-sm text-brand-muted">
            {evaluation ? 'The judge panel runs once the task completes.' : 'No judges attached to this task.'}
          </p>
        {:else if judgeRows.length > 0}
          <ul class="divide-y divide-brand-border/40 rounded-md border border-brand-border/40">
            {#each judgeRows as row (row.slug)}
              <li class="px-4 py-3" data-testid="judge-row-{row.slug}">
                {#if row.status === 'scored' && row.result}
                  {@const r = row.result}
                  {@const delta = r.point_delta}
                  <div class="flex flex-wrap items-baseline gap-x-3 gap-y-1">
                    <span class="text-[13px] font-semibold text-brand-text">{r.judge_name}</span>
                    <span class="text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Rating {r.rating}</span>
                    <span class="text-[13px] font-semibold tabular-nums {delta >= 0 ? 'text-green-600' : 'text-red-600'}">{delta >= 0 ? '+' : ''}{delta} pts</span>
                    <span class="ml-auto text-[10px] text-brand-muted">
                      {#if r.duration_ms != null}evaluated in {(r.duration_ms / 1000).toFixed(1)}s ·{/if}
                      {new Date(r.created_at).toLocaleString()}
                    </span>
                  </div>
                  {#if r.feedback}
                    <p class="mt-1.5 text-sm text-brand-text">{r.feedback}</p>
                  {/if}
                {:else}
                  <div class="flex flex-wrap items-baseline gap-x-3 gap-y-1">
                    <span class="text-[13px] font-semibold text-brand-text">{row.name}</span>
                    {#if row.status === 'running'}
                      <span class="inline-flex items-center gap-1.5 rounded-full bg-amber-100 px-2 py-0.5 text-[10px] font-semibold text-amber-700">
                        <span class="judge-spinner" aria-hidden="true"></span>Evaluating…
                      </span>
                    {:else if row.status === 'failed'}
                      <span class="rounded-full bg-red-100 px-2 py-0.5 text-[10px] font-semibold text-red-600">Failed</span>
                    {:else}
                      <span class="rounded-full bg-gray-100 px-2 py-0.5 text-[10px] font-semibold text-gray-500">Pending</span>
                    {/if}
                    {#if row.updated_at}
                      <span class="ml-auto text-[10px] text-brand-muted">{new Date(row.updated_at).toLocaleString()}</span>
                    {/if}
                  </div>
                  {#if row.status === 'failed' && row.error}
                    <p class="mt-1.5 rounded-md bg-red-50 px-3 py-2 font-mono text-[11px] text-red-700">{row.error}</p>
                  {:else if row.status === 'pending'}
                    <p class="mt-1.5 text-[11px] text-brand-muted">Runs automatically once this task completes.</p>
                  {/if}
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    {:else}
      <div role="tabpanel" data-testid="task-stats-{task.ordinal}">
        {#if !stats || stats.agents.length === 0}
          <p class="py-6 text-center text-sm text-brand-muted">No implementation statistics for this task yet.</p>
        {:else}
          <!-- Summary row -->
          <div class="mb-4 flex flex-wrap gap-2">
            <div class="rounded-md border border-brand-border/40 bg-brand-light-blue/10 px-3 py-2" title="Everything the model was fed, cache included — with prompt caching most of the input is a cache read, and the uncached remainder alone would be misleadingly small">
              <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Tokens in / out</div>
              <div class="text-[13px] font-semibold tabular-nums text-brand-text">{formatTokens(stats.input_tokens + stats.cache_read_tokens + stats.cache_write_tokens)} / {formatTokens(stats.output_tokens)}</div>
            </div>
            <div class="rounded-md border border-brand-border/40 bg-brand-light-blue/10 px-3 py-2">
              <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Cache read / write</div>
              <div class="text-[13px] font-semibold tabular-nums text-brand-text">{formatTokens(stats.cache_read_tokens)} / {formatTokens(stats.cache_write_tokens)}</div>
            </div>
            <div class="rounded-md border border-brand-border/40 bg-brand-light-blue/10 px-3 py-2">
              <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Messages (user / agent)</div>
              <div class="text-[13px] font-semibold tabular-nums text-brand-text">{stats.user_messages} / {stats.assistant_messages}</div>
            </div>
            <div class="rounded-md border border-brand-border/40 bg-brand-light-blue/10 px-3 py-2">
              <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Tool calls</div>
              <div class="text-[13px] font-semibold tabular-nums text-brand-text">{stats.tool_calls}</div>
            </div>
            {#if stats.cost != null && stats.cost > 0}
              <div class="rounded-md border border-brand-border/40 bg-brand-light-blue/10 px-3 py-2">
                <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Cost</div>
                <div class="text-[13px] font-semibold tabular-nums text-brand-text">${stats.cost.toFixed(4)}</div>
              </div>
            {/if}
            {#if statsWindowSecs != null}
              <div class="rounded-md border border-brand-border/40 bg-brand-light-blue/10 px-3 py-2">
                <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Implementation time</div>
                <div class="text-[13px] font-semibold tabular-nums text-brand-text">{Math.floor(statsWindowSecs / 60)}m {statsWindowSecs % 60}s</div>
              </div>
            {/if}
          </div>

          <!-- Per-agent-session breakdown -->
          <div class="space-y-3">
            {#each stats.agents as a (a.agent + ':' + a.agent_session_id)}
              <div class="rounded-md border border-brand-border/40 overflow-hidden">
                <div class="flex flex-wrap items-baseline gap-x-3 gap-y-1 border-b border-brand-border/40 bg-brand-light-blue/10 px-4 py-2">
                  <span class="rounded-full bg-brand-blue/10 px-2 py-0.5 text-[10px] font-semibold text-brand-blue">{a.agent}</span>
                  {#if a.model}
                    <span class="text-[12px] font-semibold text-brand-text">{a.model}</span>
                  {/if}
                  <span class="font-mono text-[10px] text-brand-muted">{a.agent_session_id}</span>
                  <span class="ml-auto text-[11px] tabular-nums text-brand-muted">
                    {formatTokens(a.input_tokens + a.cache_read_tokens + a.cache_write_tokens)} in · {formatTokens(a.output_tokens)} out{#if a.cost != null && a.cost > 0}&nbsp;· ${a.cost.toFixed(4)}{/if}
                  </span>
                </div>
                <div class="grid gap-4 px-4 py-3 sm:grid-cols-2">
                  <div>
                    <div class="mb-1.5 text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Tools ({a.tool_calls} calls)</div>
                    {#if Object.keys(a.tools).length === 0}
                      <p class="text-[12px] text-brand-muted">No tool calls recorded.</p>
                    {:else}
                      <ul class="space-y-0.5">
                        {#each sortedCounts(a.tools) as [tool, count] (tool)}
                          <li class="flex items-center justify-between text-[12px]">
                            <span class="font-mono text-brand-text">{tool}</span>
                            <span class="tabular-nums text-brand-muted">{count}</span>
                          </li>
                        {/each}
                      </ul>
                    {/if}
                  </div>
                  <div>
                    <div class="mb-1.5 text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Skills loaded</div>
                    {#if Object.keys(a.skills).length === 0}
                      <p class="text-[12px] text-brand-muted">No skills loaded.</p>
                    {:else}
                      <div class="flex flex-wrap gap-1.5">
                        {#each sortedCounts(a.skills) as [skill, count] (skill)}
                          <span class="rounded-full bg-brand-blue/10 px-2 py-0.5 text-[11px] font-semibold text-brand-blue">{skill}{count > 1 ? ` ×${count}` : ''}</span>
                        {/each}
                      </div>
                    {/if}
                  </div>
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    {/if}
    </div>
  {/if}
</div>
<style>
  /* Brief attention pulses when live data changes in the header. The
     elements are re-mounted via {#key} so the animation replays on every
     change. */
  @keyframes chip-pulse {
    0% {
      transform: scale(1);
      background-color: rgba(59, 130, 246, 0.35);
    }
    50% {
      transform: scale(1.12);
    }
    100% {
      transform: scale(1);
    }
  }
  .chip-pulse {
    animation: chip-pulse 0.6s ease-out;
  }

  @keyframes points-pulse {
    0% {
      transform: scale(1);
    }
    35% {
      transform: scale(1.3);
    }
    100% {
      transform: scale(1);
    }
  }
  .points-pulse {
    animation: points-pulse 0.5s ease-out;
  }

  .judge-spinner {
    width: 8px;
    height: 8px;
    border-radius: 9999px;
    border: 1.5px solid currentColor;
    border-top-color: transparent;
    animation: judge-spin 0.8s linear infinite;
  }
  @keyframes judge-spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .chip-pulse,
    .points-pulse {
      animation: none;
    }
    .judge-spinner {
      animation: none;
    }
  }
</style>
