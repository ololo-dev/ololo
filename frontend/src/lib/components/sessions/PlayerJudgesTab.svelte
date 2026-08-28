<script lang="ts">
  import type {
    PlayerTaskSummaryEntry,
    PlayerJudgeScoredPayload,
    PlayerJudgeStatusPayload,
    PlayerTaskEvaluation,
    PlayerArtifactRef,
    PlayerHistoryCommit,
    JudgeResultDetails,
  } from '$lib/types/arena';
  import { getJudgeResultDetails, getPlayerTaskJudgeLog } from '$lib/api';
  import CriteriaTrendChart from './CriteriaTrendChart.svelte';
  import * as HoverCard from '$lib/components/ui/hover-card';
  import { formatTokens } from '$lib/format';
  import { fileName, galleryEntries, type GalleryEntry } from '$lib/sessions/artifacts';
  import ImageLightbox from './ImageLightbox.svelte';
  import TaskChanges from './TaskChanges.svelte';
  import { ikAvatar } from '$lib/imagekit';
  import PlayerTodoChecklist from './PlayerTodoChecklist.svelte';

  type Props = {
    /** Tasks in display order; only revealed tasks should be passed in. */
    tasks: PlayerTaskSummaryEntry[];
    judgeResultsByTask: Map<string, PlayerJudgeScoredPayload[]>;
    judgeStatusesByTask: Map<string, PlayerJudgeStatusPayload[]>;
    /** Admin viewers get full judge-run details (model, tokens, log, error). */
    isAdmin?: boolean;
    /** Needed for the admin raw-log download (join code + player UUID). */
    sessionCode?: string;
    playerId?: string;
    /** Open-ended evaluation state; criteria sheets render under verdicts. */
    evaluations?: PlayerTaskEvaluation[];
    /** task_id → the task's attributed commits, for the collapsible
     *  Changes block at the top of each card. */
    changesByTask?: Map<string, { commits: PlayerHistoryCommit[]; mode: 'range' | 'per-commit' }>;
    /** judge_slug → avatar url, from the project's judges list. */
    judgeAvatars?: Record<string, string>;
    /** The session has ended — the per-judge summary table becomes worth
     *  reading (during a run it only repeats half-finished numbers). */
    sessionFinished?: boolean;
    /** Copy/paste validation result from finish, if the scan ran. */
    similarityAdjustment?: {
      note: string;
      point_delta: number;
      duplicated_pct?: number;
      sources?: { join_code: string; player: string; matched_lines: number }[];
    } | null;
  };
  let { tasks, judgeResultsByTask, judgeStatusesByTask, isAdmin = false, sessionCode = '', playerId = '', evaluations = [], judgeAvatars = {}, sessionFinished = false, changesByTask = new Map(), similarityAdjustment = null }: Props = $props();

  type CriterionCell = {
    key: string;
    title: string;
    score: number | null;
    rationale: string;
  };

  // (task_id, judge_slug) → the criteria this judge scored, from the
  // snapshot's evaluation blocks.
  const criteriaByTaskJudge = $derived.by(() => {
    const map = new Map<string, CriterionCell[]>();
    for (const ev of evaluations) {
      for (const c of ev.criteria) {
        for (const s of c.scores ?? []) {
          const k = `${ev.task_id}:${s.judge_slug}`;
          const arr = map.get(k) ?? [];
          arr.push({
            key: c.key,
            title: c.title,
            score: s.score ?? null,
            rationale: s.rationale,
          });
          map.set(k, arr);
        }
      }
    }
    return map;
  });

  // task_id → the open-ended evaluation block (TODO, criteria, chart),
  // rendered inside the task's judge card now that there is no separate tab.
  const evaluationByTask = $derived.by(() => {
    const map = new Map<string, PlayerTaskEvaluation>();
    for (const ev of evaluations) map.set(ev.task_id, ev);
    return map;
  });

  // task_id → gallery entries: probe-delivered artifacts first, then image
  // files the participant committed on their own — everything the judges
  // saw, in one strip at the top of the task card. Deduped by repo path:
  // retries of a request deliver the same file under several probe ids.
  /** ` (queued 5 min)` — how long a run has sat waiting for a slot. */
  function queuedFor(updatedAt: string | null | undefined): string {
    if (!updatedAt) return ''
    const mins = Math.floor((Date.now() - new Date(updatedAt).getTime()) / 60_000)
    return mins >= 1 ? ` (queued ${mins} min)` : ''
  }

  const artifactsByTask = $derived.by(() => {
    const map = new Map<string, GalleryEntry[]>();
    for (const ev of evaluations) {
      // Judges re-request the same capture under fresh request folders, so
      // dedupe by delivered file name and keep only the newest delivery.
      const byName = new Map<string, PlayerArtifactRef>();
      for (const a of ev.artifacts ?? []) byName.set(fileName(a.label), a);
      const entries: GalleryEntry[] = [...byName.values()].flatMap((a) =>
        galleryEntries(a, artifactUrl),
      );
      const seen = new Set([...byName.values()].map((a) => a.label));
      const seenNames = new Set(byName.keys());
      for (const p of ev.repo_images ?? []) {
        if (seen.has(p) || seenNames.has(fileName(p))) continue;
        entries.push({ key: `repo:${p}`, src: repoFileUrl(p), content_type: 'image/*', label: p });
      }
      if (entries.length > 0) map.set(ev.task_id, entries);
    }
    return map;
  });

  function artifactUrl(probeId: string, i = 0): string {
    const base = `/api/sessions/${encodeURIComponent(sessionCode)}/players/${encodeURIComponent(playerId)}/artifacts/${encodeURIComponent(probeId)}`;
    return i > 0 ? `${base}?i=${i}` : base;
  }

  function repoFileUrl(path: string): string {
    return `/api/sessions/${encodeURIComponent(sessionCode)}/players/${encodeURIComponent(playerId)}/repo-file?path=${encodeURIComponent(path)}`;
  }

  // Lightbox over one task's image gallery: track the task and the index so
  // ‹ › (and arrow keys) walk the strip without leaving fullscreen.
  let lightbox = $state<{ taskId: string; index: number } | null>(null);
  const lightboxImages = $derived(
    lightbox
      ? (artifactsByTask.get(lightbox.taskId) ?? []).filter((s) =>
          s.content_type.startsWith('image/'),
        )
      : [],
  );
  const lightboxShot = $derived(lightbox ? (lightboxImages[lightbox.index] ?? null) : null);

  function openLightbox(taskId: string, key: string) {
    const images = (artifactsByTask.get(taskId) ?? []).filter((s) =>
      s.content_type.startsWith('image/'),
    );
    const index = images.findIndex((s) => s.key === key);
    if (index >= 0) lightbox = { taskId, index };
  }

  function stepLightbox(delta: number) {
    if (!lightbox || lightboxImages.length === 0) return;
    const n = lightboxImages.length;
    lightbox = { ...lightbox, index: (lightbox.index + delta + n) % n };
  }

  type JudgeRow = {
    slug: string;
    name: string;
    status: string;
    error?: string | null;
    updated_at?: string | null;
    result?: PlayerJudgeScoredPayload;
    judge_result_id?: string | null;
  };

  // Admin: download a task's raw judge-log file (all judges' telemetry).
  let rawLogBusyTask = $state<string | null>(null);
  let rawLogErrors = $state<Record<string, string>>({});
  async function downloadRawJudgeLog(task: PlayerTaskSummaryEntry) {
    if (!sessionCode || !playerId) return;
    rawLogBusyTask = task.task_id;
    rawLogErrors = { ...rawLogErrors, [task.task_id]: '' };
    try {
      const doc = await getPlayerTaskJudgeLog(sessionCode, playerId, task.task_id);
      const blob = new Blob([JSON.stringify(doc, null, 2)], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `judge-log-${sessionCode}-task${task.ordinal}.json`;
      a.click();
      URL.revokeObjectURL(url);
    } catch {
      rawLogErrors = { ...rawLogErrors, [task.task_id]: 'No log file for this task yet.' };
    } finally {
      rawLogBusyTask = null;
    }
  }

  // Admin-only judge-run details, fetched lazily per judge_result_id.
  let judgeDetailsOpen = $state(new Set<string>());
  let judgeDetails = $state<Record<string, JudgeResultDetails | 'loading' | 'error'>>({});
  async function toggleJudgeDetails(id: string) {
    const next = new Set(judgeDetailsOpen);
    if (next.has(id)) {
      next.delete(id);
      judgeDetailsOpen = next;
      return;
    }
    next.add(id);
    judgeDetailsOpen = next;
    if (judgeDetails[id] && judgeDetails[id] !== 'error') return;
    judgeDetails = { ...judgeDetails, [id]: 'loading' };
    try {
      const d = await getJudgeResultDetails(id);
      judgeDetails = { ...judgeDetails, [id]: d };
    } catch {
      judgeDetails = { ...judgeDetails, [id]: 'error' };
    }
  }
  // Events sorted by start time: the enveloping LLM event is recorded when
  // the agent loop finishes but stamped with its start, so raw array order
  // puts it after the tool calls it contains.
  function sortedLog(d: JudgeResultDetails) {
    return [...(d.run_log ?? [])].sort((a, b) => a.at_ms - b.at_ms);
  }
  function logOffset(d: JudgeResultDetails, at: number): string {
    const log = d.run_log ?? [];
    const first = log.length > 0 ? Math.min(...log.map((e) => e.at_ms)) : at;
    return ((at - first) / 1000).toFixed(1) + 's';
  }

  // Same join as PlayerTaskPanel: lifecycle rows carry the verdict once scored,
  // and legacy sessions with verdicts but no lifecycle rows synthesize them.
  function rowsFor(taskId: string): JudgeRow[] {
    const results = judgeResultsByTask.get(taskId) ?? [];
    const statuses = judgeStatusesByTask.get(taskId) ?? [];
    if (statuses.length === 0) {
      return results.map((r) => ({
        slug: r.judge_slug,
        name: r.judge_name,
        status: 'scored',
        result: r,
      }));
    }
    const bySlug = new Map(results.map((r) => [r.judge_slug, r]));
    return statuses.map((s) => ({
      slug: s.judge_slug,
      name: s.judge_name,
      status: s.status,
      error: s.error,
      updated_at: s.updated_at,
      result: s.status === 'scored' ? bySlug.get(s.judge_slug) : undefined,
      judge_result_id: s.judge_result_id,
    }));
  }

  // A run denied by the account's monthly judge quota is not a judging
  // outcome — listing each one as a failed judge buries the one fact that
  // matters. They collapse into a single banner above the cards instead.
  const QUOTA_MARKER = 'Monthly judge-run limit';
  function isQuotaRow(row: { status: string; error?: string | null }): boolean {
    return row.status === 'failed' && (row.error ?? '').includes(QUOTA_MARKER);
  }

  /** Judge runs skipped by the quota across all tasks (hidden from lists). */
  const quotaSkippedCount = $derived(
    [...tasks].flatMap((t) => rowsFor(t.task_id)).filter(isQuotaRow).length,
  );

  // A task earns a card when judges touched it OR it carries an open-ended
  // evaluation — the latter must be visible before the first verdict lands.
  // Newest task first: the reader wants the latest verdicts on top.
  const taskRows = $derived(
    [...tasks]
      .sort((a, b) => b.ordinal - a.ordinal)
      .map((t) => ({ task: t, rows: rowsFor(t.task_id).filter((r) => !isQuotaRow(r)) }))
      .filter((g) => g.rows.length > 0 || evaluationByTask.has(g.task.task_id)),
  );

  const allVerdicts = $derived(
    taskRows.flatMap((g) => g.rows.flatMap((r) => (r.result ? [r.result] : []))),
  );

  const totalPoints = $derived(
    allVerdicts.reduce((sum, r) => sum + r.point_delta, 0),
  );

  const avgRating = $derived(
    allVerdicts.length > 0
      ? allVerdicts.reduce((sum, r) => sum + r.rating, 0) / allVerdicts.length
      : null,
  );

  // Per-judge aggregate across every task it ran on — the closing summary.
  const byJudge = $derived.by(() => {
    const map = new Map<string, { name: string; count: number; points: number; ratingSum: number }>();
    for (const r of allVerdicts) {
      const agg = map.get(r.judge_slug) ?? { name: r.judge_name, count: 0, points: 0, ratingSum: 0 };
      agg.count += 1;
      agg.points += r.point_delta;
      agg.ratingSum += r.rating;
      map.set(r.judge_slug, agg);
    }
    return [...map.entries()].sort((a, b) => a[1].name.localeCompare(b[1].name));
  });

  const pendingCount = $derived(
    taskRows.flatMap((g) => g.rows).filter((r) => r.status === 'pending' || r.status === 'running')
      .length,
  );

</script>

<div data-testid="player-judges-tab">
  {#snippet criterionRow(cell: CriterionCell)}
    <div class="flex items-baseline justify-between gap-3 text-left">
      <span class="text-[12px] font-semibold text-brand-text">{cell.title}</span>
      {#if cell.score != null}
        <span
          class="text-[13px] font-bold tabular-nums {cell.score >= 7
            ? 'text-green-600'
            : cell.score >= 4
              ? 'text-amber-600'
              : 'text-red-600'}"
        >{cell.score.toFixed(1)}<span class="font-normal text-brand-muted">/10</span></span>
      {:else}
        <span class="text-[12px] text-brand-muted">not assessed</span>
      {/if}
    </div>
    {#if cell.score != null}
      <div class="mt-1 h-1.5 overflow-hidden rounded bg-brand-border/30">
        <div
          class="h-full rounded {cell.score >= 7 ? 'bg-green-500' : cell.score >= 4 ? 'bg-amber-500' : 'bg-red-500'}"
          style="width: {(cell.score / 10) * 100}%"
        ></div>
      </div>
    {/if}
  {/snippet}

  {#snippet judgeAvatar(slug: string, name: string, size: number)}
    {#if judgeAvatars[slug]}
      <img
        src={ikAvatar(judgeAvatars[slug], size)}
        alt="{name} avatar"
        style="width: {size}px; height: {size}px;"
        class="shrink-0 rounded-full object-cover"
        data-testid="judge-avatar-{slug}"
      />
    {:else}
      <span
        style="width: {size}px; height: {size}px; font-size: {Math.max(11, Math.round(size * 0.38))}px;"
        class="inline-flex shrink-0 items-center justify-center rounded-full bg-brand-blue/15 font-semibold text-brand-blue"
      >{name.charAt(0).toUpperCase()}</span>
    {/if}
  {/snippet}

  {#if taskRows.length === 0}
    <div class="rounded-[8px] bg-white py-16 text-center text-sm text-brand-muted shadow-sm">
      No judge verdicts yet. Judges run automatically once a task completes.
    </div>
  {:else}
    <!-- Overall summary -->
    <div class="mb-4 flex flex-wrap gap-2">
      <div class="rounded-md border border-brand-border/40 bg-white px-3 py-2 shadow-sm">
        <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Verdicts</div>
        <div class="text-[15px] font-semibold tabular-nums text-brand-text">{allVerdicts.length}</div>
      </div>
      <div class="rounded-md border border-brand-border/40 bg-white px-3 py-2 shadow-sm">
        <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Judge points</div>
        <div
          data-testid="judges-total-points"
          class="text-[15px] font-semibold tabular-nums {totalPoints >= 0 ? 'text-green-600' : 'text-red-600'}"
        >{totalPoints >= 0 ? '+' : ''}{totalPoints}</div>
      </div>
      {#if avgRating !== null}
        <div class="rounded-md border border-brand-border/40 bg-white px-3 py-2 shadow-sm">
          <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Avg rating</div>
          <div class="text-[15px] font-semibold tabular-nums text-brand-text">{avgRating.toFixed(1)}</div>
        </div>
      {/if}
      {#if pendingCount > 0}
        <div class="rounded-md border border-brand-border/40 bg-white px-3 py-2 shadow-sm">
          <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">In flight</div>
          <div class="text-[15px] font-semibold tabular-nums text-amber-700">{pendingCount}</div>
        </div>
      {/if}
    </div>

    <!-- How the panel's per-criterion scores moved task by task; renders
         itself away on projects without criteria sheets. -->
    <CriteriaTrendChart {tasks} {evaluations} />

    {#if sessionFinished && byJudge.length > 1}
      <!-- The closing summary: how each judge scored across the session. -->
      <div class="mb-4 overflow-x-auto rounded-[8px] bg-white shadow-sm" data-testid="judges-summary-table">
        <table class="w-full text-[12px]">
          <thead>
            <tr class="border-b border-brand-border/40 text-left text-[10px] uppercase tracking-wider text-brand-muted">
              <th class="px-4 py-2">Judge</th>
              <th class="px-4 py-2 text-right">Runs</th>
              <th class="px-4 py-2 text-right">Avg rating</th>
              <th class="px-4 py-2 text-right">Points</th>
            </tr>
          </thead>
          <tbody>
            {#each byJudge as [slug, agg] (slug)}
              <tr class="border-b border-brand-border/20 last:border-0">
                <td class="px-4 py-2 font-semibold text-brand-text">
                  <span class="flex items-center gap-2.5">
                    {@render judgeAvatar(slug, agg.name, 28)}
                    {agg.name}
                  </span>
                </td>
                <td class="px-4 py-2 text-right tabular-nums text-brand-muted">{agg.count}</td>
                <td class="px-4 py-2 text-right tabular-nums text-brand-text">{(agg.ratingSum / agg.count).toFixed(1)}</td>
                <td class="px-4 py-2 text-right tabular-nums font-semibold {agg.points >= 0 ? 'text-green-600' : 'text-red-600'}">{agg.points >= 0 ? '+' : ''}{agg.points}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}

    {#if similarityAdjustment}
      {@const penalized = similarityAdjustment.point_delta < 0}
      <div
        class="mb-4 rounded-[10px] border px-4 py-3 {penalized
          ? 'border-red-200 bg-red-50'
          : 'border-green-200 bg-green-50'}"
        data-testid="judges-similarity-adjustment"
      >
        <div class="flex flex-wrap items-center gap-x-3 gap-y-1">
          <div class="min-w-0 flex-1 text-[13px] {penalized ? 'text-red-800' : 'text-green-800'}">
            <span class="font-semibold">Copy/paste check:</span>
            {similarityAdjustment.note}{penalized
              ? ' — repeat runs are expected to be built fresh.'
              : '.'}
          </div>
          {#if penalized}
            <span class="shrink-0 text-[15px] font-bold tabular-nums text-red-700">
              {similarityAdjustment.point_delta} pts
            </span>
          {/if}
        </div>
        <!-- A passed check names nobody: sources are shown only with a penalty. -->
        {#if penalized && (similarityAdjustment.sources ?? []).length > 0}
          <div class="mt-2 flex flex-wrap gap-2">
            {#each similarityAdjustment.sources ?? [] as src (src.join_code + src.player)}
              <a
                href="/s/{src.join_code}"
                class="inline-flex items-baseline gap-1.5 rounded-full bg-white/70 px-2.5 py-0.5 text-[11px] text-red-800 no-underline hover:underline"
              >
                <span class="font-semibold">{src.player}</span>
                <span class="font-mono">{src.join_code}</span>
                <span class="opacity-70">{src.matched_lines} lines</span>
              </a>
            {/each}
          </div>
        {/if}
      </div>
    {/if}

    {#if quotaSkippedCount > 0}
      <!-- Quota-denied runs collapse into one banner: the reason matters,
           the per-judge repetition of it does not. -->
      <div
        class="mb-4 flex flex-wrap items-center gap-x-3 gap-y-2 rounded-[10px] border border-amber-200 bg-amber-50 px-4 py-3"
        data-testid="judges-quota-banner"
      >
        <svg class="h-5 w-5 shrink-0 text-amber-500" viewBox="0 0 24 24" fill="none" aria-hidden="true">
          <path d="M12 9v4m0 4h.01M10.3 3.9 1.8 18.1A2 2 0 0 0 3.5 21h17a2 2 0 0 0 1.7-2.9L13.7 3.9a2 2 0 0 0-3.4 0z"
                stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
        <div class="min-w-0 flex-1 text-[13px] text-amber-900">
          <span class="font-semibold">
            Reached the monthly limit of judge evaluations
          </span>
          — {quotaSkippedCount} {quotaSkippedCount === 1 ? 'review was' : 'reviews were'} skipped
          this session. Reviews resume on the 1st of next month (UTC).
        </div>
      </div>
    {/if}

    <!-- One card per task: its judge panel, then the evidence and evaluation. -->
    <div class="space-y-5">
      {#each taskRows as group (group.task.task_id)}
        {@const ev = evaluationByTask.get(group.task.task_id) ?? null}
        {@const shots = artifactsByTask.get(group.task.task_id) ?? []}
        {@const changes = changesByTask.get(group.task.task_id) ?? null}
        <section class="overflow-hidden rounded-[10px] bg-white shadow-sm" data-testid="judges-task-{group.task.ordinal}">
          <header class="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-brand-border/40 px-4 py-3 sm:px-6">
            <span class="font-mono text-[11px] text-brand-blue/70">#{group.task.ordinal}</span>
            <h3 class="min-w-0 flex-1 truncate font-heading text-[14px] font-semibold text-brand-text">{group.task.title}</h3>
            {#if ev?.deadline_at}
              <span class="hidden text-[11px] text-brand-muted sm:inline">
                window closes {new Date(ev.deadline_at).toLocaleTimeString()}
              </span>
            {/if}
            <span class="text-[11px] tabular-nums text-brand-muted">
              {group.rows.filter((r) => r.status === 'scored').length}/{group.rows.length} scored
            </span>
            {#if isAdmin && sessionCode && playerId}
              {#if rawLogErrors[group.task.task_id]}
                <span class="text-[11px] text-brand-muted">{rawLogErrors[group.task.task_id]}</span>
              {/if}
              <button
                type="button"
                data-testid="download-judge-log-{group.task.ordinal}"
                onclick={() => void downloadRawJudgeLog(group.task)}
                disabled={rawLogBusyTask === group.task.task_id}
                class="rounded px-2 py-0.5 text-[11px] font-semibold text-brand-blue transition-colors hover:bg-brand-blue/10 disabled:opacity-40"
              >{rawLogBusyTask === group.task.task_id ? 'Fetching…' : 'Raw log'}</button>
            {/if}
          </header>

          <div class="space-y-6 px-4 py-4 sm:px-6 sm:py-5">
            {#if changes && changes.commits.length > 0}
              <!-- The task's diff, folded by default: the evidence and the
                   verdicts are the story; the code is a click away. -->
              <details
                class="group/changes rounded-md border border-brand-border/40"
                data-testid="judges-changes-{group.task.ordinal}"
              >
                <summary
                  class="flex cursor-pointer select-none items-center gap-2 px-4 py-2.5 text-[12px] font-semibold text-brand-text hover:bg-brand-light-blue/20"
                >
                  <svg width="10" height="10" viewBox="0 0 24 24" fill="none" aria-hidden="true" class="shrink-0 text-brand-muted transition-transform group-open/changes:rotate-90"><path d="M9 18l6-6-6-6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>
                  Changes
                  <span class="font-normal text-brand-muted">
                    · {changes.commits.length} {changes.commits.length === 1 ? 'commit' : 'commits'}
                  </span>
                </summary>
                <div class="border-t border-brand-border/40 px-4 py-3">
                  <TaskChanges
                    commits={changes.commits}
                    mode={changes.mode}
                    testid="judges-changes-diff-{group.task.ordinal}"
                  />
                </div>
              </details>
            {/if}

            {#if shots.length > 0 && sessionCode && playerId}
              <!-- The evidence first: every capture the judges saw — requested
                   artifacts and self-committed images — in one strip. -->
              <div>
                <h4 class="mb-2 font-heading text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Screenshots</h4>
                <div
                  class="flex gap-3 overflow-x-auto pb-1"
                  data-testid="judge-screenshots-{group.task.ordinal}"
                >
                  {#each shots as shot (shot.key)}
                    {#if shot.content_type.startsWith('image/')}
                      <button
                        type="button"
                        class="group/shot shrink-0 overflow-hidden rounded-lg border border-brand-border/60 transition-shadow hover:shadow-md"
                        title="{shot.label} — click to enlarge"
                        data-testid="judge-screenshot-{shot.key}"
                        onclick={() => openLightbox(group.task.task_id, shot.key)}
                      >
                        <img
                          src={shot.src}
                          alt={shot.label}
                          class="h-28 w-auto cursor-zoom-in object-cover transition-transform group-hover/shot:scale-105 sm:h-32"
                          loading="lazy"
                        />
                      </button>
                    {:else if shot.content_type.startsWith('video/')}
                      <!-- Playable in place, fullscreen via native controls. -->
                      <!-- svelte-ignore a11y_media_has_caption -->
                      <video
                        src={shot.src}
                        controls
                        preload="metadata"
                        title={shot.label}
                        data-testid="judge-screencast-{shot.key}"
                        class="h-28 shrink-0 rounded-lg border border-brand-border/60 sm:h-32"
                      ></video>
                    {:else}
                      <a
                        href={shot.src}
                        class="self-center whitespace-nowrap text-[12px] font-semibold text-brand-blue underline"
                        download
                      >
                        Download {shot.label}
                      </a>
                    {/if}
                  {/each}
                </div>
              </div>
            {/if}

            {#if ev}
              <!-- The plan next: what the player set out to do frames every
                   verdict below it. -->
              <div data-testid="judge-evaluation-{group.task.ordinal}">
                <PlayerTodoChecklist evaluation={ev} />
              </div>
            {/if}

            {#if group.rows.length > 0}
              <!-- The judge panel as a review thread: avatar on the left,
                   the verdict as a review bubble — feedback gets a full
                   paragraph, criteria read as score bars, and every state
                   (queued, evaluating, failed) uses the same shape. -->
              <div class="space-y-4">
                {#each group.rows as row (row.slug)}
                  {@const r = row.result}
                  <div class="flex items-start gap-3" data-testid="judge-verdict-{group.task.ordinal}-{row.slug}">
                    <div class="mt-1 hidden sm:block">
                      {@render judgeAvatar(row.slug, row.name, 44)}
                    </div>
                    <div class="min-w-0 flex-1 overflow-hidden rounded-xl border border-brand-border/60">
                      <div class="flex flex-wrap items-center gap-x-2.5 gap-y-1 border-b border-brand-border/40 bg-brand-light-blue/20 px-4 py-2.5">
                        <span class="sm:hidden">{@render judgeAvatar(row.slug, row.name, 28)}</span>
                        <span class="text-[14px] font-semibold text-brand-text">{row.name}</span>
                        {#if r}
                          <span class="text-[11px] text-brand-muted">
                            {#if r.duration_ms != null}evaluated in {(r.duration_ms / 1000).toFixed(1)}s · {/if}{new Date(r.created_at).toLocaleTimeString()}
                          </span>
                          <span
                            class="ml-auto shrink-0 rounded-full px-3 py-0.5 text-[14px] font-bold tabular-nums {r.point_delta >= 0 ? 'bg-green-100 text-green-700' : 'bg-red-100 text-red-600'}"
                          >{r.point_delta >= 0 ? '+' : ''}{r.point_delta} pts</span>
                        {:else if row.status === 'running'}
                          <span class="ml-auto inline-flex shrink-0 items-center gap-1.5 rounded-full bg-amber-100 px-2.5 py-0.5 text-[11px] font-semibold text-amber-700">
                            <span class="inline-block h-[7px] w-[7px] animate-pulse rounded-full bg-amber-500"></span>
                            Evaluating…
                          </span>
                        {:else if row.status === 'failed'}
                          <span class="ml-auto shrink-0 rounded-full bg-red-100 px-2.5 py-0.5 text-[11px] font-semibold text-red-600">Failed</span>
                        {:else}
                          <span class="ml-auto shrink-0 rounded-full bg-gray-100 px-2.5 py-0.5 text-[11px] font-semibold text-gray-500">Queued</span>
                        {/if}
                      </div>

                      <div class="space-y-3 px-4 py-3">
                        {#if r?.feedback}
                          <p class="text-[13px] leading-relaxed text-brand-text">{r.feedback}</p>
                        {/if}

                        {#if r}
                          {@const cells = criteriaByTaskJudge.get(`${group.task.task_id}:${row.slug}`) ?? []}
                          {#if cells.length > 0}
                            <!-- The judge's criteria sheet as score bars. -->
                            <div class="space-y-2.5" data-testid="judge-criteria-{group.task.ordinal}-{row.slug}">
                              {#each cells as cell (cell.key)}
                                <!-- The rationale lives in a hovercard: hover
                                     or focus the criterion row to read it. The
                                     card floats in a portal so the bubble's
                                     rounded-corner overflow clipping cannot
                                     cut it off. -->
                                {#if cell.rationale}
                                  <HoverCard.Root openDelay={150}>
                                    <HoverCard.Trigger class="block w-full cursor-help no-underline">
                                      {@render criterionRow(cell)}
                                    </HoverCard.Trigger>
                                    <HoverCard.Content
                                      class="w-max max-w-[360px] text-[12px] leading-snug text-brand-muted"
                                      align="start"
                                      sideOffset={4}
                                      data-testid="criterion-rationale-hovercard"
                                    >
                                      {cell.rationale}
                                    </HoverCard.Content>
                                  </HoverCard.Root>
                                {:else}
                                  {@render criterionRow(cell)}
                                {/if}
                              {/each}
                            </div>
                          {/if}
                        {:else if row.status === 'running'}
                          <p class="text-[12px] text-brand-muted">Reading the committed code, probe measurements, and screenshots…</p>
                        {:else if row.status === 'failed' && row.error}
                          <p class="rounded-md bg-red-50 px-3 py-2 font-mono text-[11px] text-red-700">{row.error}</p>
                        {:else if row.status !== 'failed'}
                          <!-- "Queued" means two different things: before the delivery
                               the panel simply has nothing to read yet; after it (some
                               sibling already scored or is running, or the task result
                               landed) this judge is waiting for a queue slot. -->
                          {@const delivered =
                            group.task.result?.status === 'completed' ||
                            group.task.result?.status === 'correct' ||
                            group.rows.some((r2) => r2.status === 'scored' || r2.status === 'running')}
                          {#if delivered}
                            <p class="text-[12px] text-brand-muted">
                              In line for a judge slot{queuedFor(row.updated_at)} — the delivery has landed, other judges run first.
                            </p>
                          {:else}
                            <p class="text-[12px] text-brand-muted">Starts once the task is delivered and its snapshot lands.</p>
                          {/if}
                        {/if}

                        {#if row.updated_at && !r}
                          <p class="text-[10px] text-brand-muted">{new Date(row.updated_at).toLocaleString()}</p>
                        {/if}

{#if isAdmin && row.judge_result_id}
                      {@const rid = row.judge_result_id}
                      <button
                        type="button"
                        data-testid="judge-details-toggle-{group.task.ordinal}-{row.slug}"
                        onclick={() => toggleJudgeDetails(rid)}
                        class="mt-2 rounded px-2 py-0.5 text-[11px] font-semibold text-brand-blue transition-colors hover:bg-brand-blue/10"
                      >{judgeDetailsOpen.has(rid) ? 'Hide run details' : 'Run details (admin)'}</button>
                      {#if judgeDetailsOpen.has(rid)}
                        {@const d = judgeDetails[rid]}
                        <div class="mt-2 rounded-md border border-brand-border/40 bg-brand-light-blue/10 px-3 py-2" data-testid="judge-details-{group.task.ordinal}-{row.slug}">
                          {#if d === 'loading' || d === undefined}
                            <p class="text-[11px] text-brand-muted">Loading run details…</p>
                          {:else if d === 'error'}
                            <p class="text-[11px] text-red-600">Could not load run details.</p>
                          {:else}
                            <div class="flex flex-wrap gap-2">
                              <div class="rounded-md border border-brand-border/40 bg-white px-2.5 py-1.5">
                                <div class="text-[9px] font-semibold uppercase tracking-wider text-brand-muted">Model</div>
                                <div class="text-[12px] font-semibold text-brand-text">{d.provider ? d.provider + '/' : ''}{d.model || '—'}</div>
                              </div>
                              <div class="rounded-md border border-brand-border/40 bg-white px-2.5 py-1.5">
                                <div class="text-[9px] font-semibold uppercase tracking-wider text-brand-muted">Tokens in / out</div>
                                <div class="text-[12px] font-semibold tabular-nums text-brand-text">{formatTokens(d.tokens_input)} / {formatTokens(d.tokens_output)}</div>
                              </div>
                              <div class="rounded-md border border-brand-border/40 bg-white px-2.5 py-1.5">
                                <div class="text-[9px] font-semibold uppercase tracking-wider text-brand-muted">Duration</div>
                                <div class="text-[12px] font-semibold tabular-nums text-brand-text">{d.duration_ms != null ? (d.duration_ms / 1000).toFixed(1) + 's' : '—'}</div>
                              </div>
                              <div class="rounded-md border border-brand-border/40 bg-white px-2.5 py-1.5">
                                <div class="text-[9px] font-semibold uppercase tracking-wider text-brand-muted">Status</div>
                                <div class="text-[12px] font-semibold text-brand-text">{d.status}</div>
                              </div>
                              {#if d.judge_fingerprint}
                                <div class="rounded-md border border-brand-border/40 bg-white px-2.5 py-1.5" title="Fingerprint of the judge definition that produced this verdict">
                                  <div class="text-[9px] font-semibold uppercase tracking-wider text-brand-muted">Judge build</div>
                                  <div class="font-mono text-[12px] font-semibold text-brand-text">{d.judge_fingerprint}</div>
                                </div>
                              {/if}
                            </div>
                            {#if d.evidence}
                              <details class="mt-2">
                                <summary class="cursor-pointer text-[9px] font-semibold uppercase tracking-wider text-brand-muted">Evidence the judge read</summary>
                                <pre class="mt-1 max-h-72 overflow-auto whitespace-pre-wrap rounded-md border border-brand-border/40 bg-white px-3 py-2 font-mono text-[11px] text-brand-text">{JSON.stringify(d.evidence, null, 2)}</pre>
                              </details>
                            {/if}
                            {#if d.error}
                              <div class="mt-2">
                                <div class="text-[9px] font-semibold uppercase tracking-wider text-brand-muted">Full error (admin only)</div>
                                <pre class="mt-1 max-h-40 overflow-auto whitespace-pre-wrap rounded-md bg-red-50 px-3 py-2 font-mono text-[11px] text-red-700">{d.error}</pre>
                              </div>
                            {/if}
                            {#if d.run_log && d.run_log.length > 0}
                              <div class="mt-2">
                                <div class="text-[9px] font-semibold uppercase tracking-wider text-brand-muted">Run log ({d.run_log.length} events)</div>
                                <div class="mt-1 max-h-56 overflow-auto rounded-md border border-brand-border/40 bg-white">
                                  <table class="w-full text-[11px]">
                                    <thead>
                                      <tr class="border-b border-brand-border/40 text-left text-[9px] uppercase tracking-wider text-brand-muted">
                                        <th class="px-2 py-1">t+</th>
                                        <th class="px-2 py-1">Event</th>
                                        <th class="px-2 py-1 text-right">Duration</th>
                                        <th class="px-2 py-1 text-right">Tokens in/out</th>
                                        <th class="px-2 py-1 text-right">Output</th>
                                      </tr>
                                    </thead>
                                    <tbody>
                                      {#each sortedLog(d) as ev, i (i)}
                                        <tr class="border-b border-brand-border/20 last:border-0 align-top {ev.error ? 'bg-red-50' : ''}">
                                          <td class="px-2 py-1 tabular-nums text-brand-muted">{logOffset(d, ev.at_ms)}</td>
                                          <td class="px-2 py-1">
                                            <span class="font-semibold text-brand-text">{ev.kind === 'tool' ? (ev.name ?? 'tool') : 'LLM turn'}</span>
                                            {#if ev.kind !== 'tool' && ev.model}
                                              <span class="ml-1 text-[10px] text-brand-muted">{ev.model}</span>
                                            {/if}
                                            {#if ev.args}
                                              <div class="mt-0.5 break-all font-mono text-[10px] text-brand-muted">{ev.args}</div>
                                            {/if}
                                            {#if ev.error}
                                              <div class="mt-0.5 break-all font-mono text-[10px] text-red-700">{ev.error}</div>
                                            {/if}
                                            {#if ev.input || ev.output || ev.messages}
                                              <details class="mt-1">
                                                <summary class="cursor-pointer text-[10px] font-semibold text-brand-blue">telemetry</summary>
                                                {#if ev.input}
                                                  <div class="mt-1 text-[9px] font-semibold uppercase tracking-wider text-brand-muted">Prompt (gen_ai.prompt)</div>
                                                  <pre class="mt-0.5 max-h-40 overflow-auto whitespace-pre-wrap rounded bg-brand-light-blue/20 px-2 py-1 font-mono text-[10px] text-brand-text">{ev.input}</pre>
                                                {/if}
                                                {#if ev.output}
                                                  <div class="mt-1 text-[9px] font-semibold uppercase tracking-wider text-brand-muted">{ev.kind === 'tool' ? 'Tool result' : 'Completion (gen_ai.completion)'}</div>
                                                  <pre class="mt-0.5 max-h-40 overflow-auto whitespace-pre-wrap rounded bg-brand-light-blue/20 px-2 py-1 font-mono text-[10px] text-brand-text">{ev.output}</pre>
                                                {/if}
                                                {#if ev.messages}
                                                  <div class="mt-1 text-[9px] font-semibold uppercase tracking-wider text-brand-muted">Turn transcript ({Array.isArray(ev.messages) ? ev.messages.length : '?'} messages)</div>
                                                  <pre class="mt-0.5 max-h-56 overflow-auto whitespace-pre-wrap rounded bg-brand-light-blue/20 px-2 py-1 font-mono text-[10px] text-brand-text">{JSON.stringify(ev.messages, null, 2)}</pre>
                                                {/if}
                                              </details>
                                            {/if}
                                          </td>
                                          <td class="px-2 py-1 text-right tabular-nums">{(ev.duration_ms / 1000).toFixed(1)}s</td>
                                          <td class="px-2 py-1 text-right tabular-nums">
                                            {ev.tokens_input != null || ev.tokens_output != null ? `${formatTokens(ev.tokens_input)} / ${formatTokens(ev.tokens_output)}` : '—'}
                                            {#if (ev.tokens_cache_read ?? 0) > 0 || (ev.tokens_cache_write ?? 0) > 0}
                                              <div class="text-[9px] text-brand-muted">cache {formatTokens(ev.tokens_cache_read)} / {formatTokens(ev.tokens_cache_write)}</div>
                                            {/if}
                                          </td>
                                          <td class="px-2 py-1 text-right tabular-nums">{ev.output_chars != null ? ev.output_chars + ' ch' : '—'}</td>
                                        </tr>
                                      {/each}
                                    </tbody>
                                  </table>
                                </div>
                              </div>
                            {/if}
                            {#if d.raw_output}
                              <details class="mt-2">
                                <summary class="cursor-pointer text-[11px] font-semibold text-brand-blue">Raw verdict output</summary>
                                <pre class="mt-1 max-h-40 overflow-auto whitespace-pre-wrap rounded-md bg-white px-3 py-2 font-mono text-[11px] text-brand-text">{d.raw_output}</pre>
                              </details>
                            {/if}
                          {/if}
                        </div>
                      {/if}
                    {/if}
                      </div>
                    </div>
                  </div>
                {/each}
              </div>
            {/if}

          </div>
        </section>
      {/each}
    </div>
  {/if}

  {#if lightboxShot}
    <ImageLightbox
      src={lightboxShot.src}
      alt={lightboxShot.label}
      counter={lightboxImages.length > 1
        ? `${(lightbox?.index ?? 0) + 1} / ${lightboxImages.length}`
        : null}
      onprev={lightboxImages.length > 1 ? () => stepLightbox(-1) : null}
      onnext={lightboxImages.length > 1 ? () => stepLightbox(1) : null}
      onclose={() => (lightbox = null)}
    />
  {/if}
</div>
