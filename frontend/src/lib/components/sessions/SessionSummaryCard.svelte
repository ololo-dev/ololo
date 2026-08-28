<script lang="ts">
  import * as HoverCard from '$lib/components/ui/hover-card';
  import { ikAvatar } from "$lib/imagekit";
  import { pointsChip, type SessionSummary } from "$lib/sessions/session-summary";

  let {
    summary,
    score,
    rank,
    totalTasks,
    taskCount,
    similarityAdjustment = null,
    judgeAvatars = {},
    testid = "session-summary",
    inDocument = false,
  }: {
    summary: SessionSummary;
    score: number;
    rank: number;
    /** Tasks in the project; falls back to how many the player was shown. */
    totalTasks: number;
    taskCount: number;
    similarityAdjustment?: {
      note: string;
      point_delta: number;
      duplicated_pct?: number;
      sources?: { join_code: string; player: string; matched_lines: number }[];
    } | null;
    judgeAvatars?: Record<string, string>;
    /** Distinguishes the chat's card from the report's in the DOM. */
    testid?: string;
    /** Sitting above the report's article rather than closing the chat: take
     *  the article's full width and its 8px corner, so the two read as one
     *  page instead of a narrow card floating over a wide block. */
    inDocument?: boolean;
  } = $props();
</script>

<!-- The final word: where the points came from, at a glance. -->
<div
  class="mx-auto w-full bg-white p-5 {inDocument
    ? 'rounded-[8px]'
    : 'max-w-[640px] rounded-2xl shadow-sm'}"
  data-testid={testid}
>
  <div class="flex items-baseline justify-between gap-3">
    <span class="text-[12px] font-bold uppercase tracking-wider text-brand-muted">Session summary</span>
    <span class="flex items-baseline gap-2">
      {#if rank > 0}
        <span class="rounded-full bg-brand-light-blue px-2 py-0.5 text-[12px] font-bold text-brand-blue">#{rank}</span>
      {/if}
      <span class="text-[26px] font-bold tabular-nums {score >= 0 ? 'text-green-600' : 'text-brand-red'}">{score}</span>
      <span class="text-[12px] text-brand-muted">pts</span>
    </span>
  </div>

  <!-- Zero tiles are projects where that mechanic does not score
       (open-ended runs award via judges + bonus) — omit, not "0". -->
  <div class="mt-3 grid grid-cols-2 gap-2 sm:grid-cols-4">
    <div class="rounded-[8px] bg-brand-light-blue/60 px-3 py-2">
      <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Tasks</div>
      <div class="text-[16px] font-bold tabular-nums text-brand-text">{summary.tasksPassed}/{totalTasks || taskCount}</div>
    </div>
    <div class="rounded-[8px] bg-brand-light-blue/60 px-3 py-2">
      <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Probes</div>
      <div class="text-[16px] font-bold tabular-nums text-brand-text">{summary.probesRun}</div>
    </div>
    {#if summary.checkPoints !== 0}
      <div class="rounded-[8px] bg-brand-light-blue/60 px-3 py-2" title="Points scored by ololo's automated checks">
        <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Check pts</div>
        <div class="text-[16px] font-bold tabular-nums {summary.checkPoints >= 0 ? 'text-green-600' : 'text-brand-red'}">{pointsChip(summary.checkPoints)}</div>
      </div>
    {/if}
    {#if summary.bonusPoints !== 0}
      <div class="rounded-[8px] bg-brand-light-blue/60 px-3 py-2" title="Completion bonuses for delivered tasks">
        <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Bonus</div>
        <div class="text-[16px] font-bold tabular-nums text-brand-text">{pointsChip(summary.bonusPoints)}</div>
      </div>
    {/if}
  </div>

  {#if similarityAdjustment}
    <!-- The copy/paste verdict, source named: a score change must
         never be reasonless (and a clean check deserves its tick). -->
    {@const penalized = similarityAdjustment.point_delta < 0}
    <!-- A passed check names nobody: the source link needs a penalty. -->
    {@const topSrc = penalized ? (similarityAdjustment.sources ?? [])[0] : undefined}
    <div
      class="mt-3 flex flex-wrap items-center gap-x-3 gap-y-1 rounded-[8px] px-3 py-2 {penalized
        ? 'bg-red-50'
        : 'bg-green-50'}"
      data-testid="{testid}-similarity"
    >
      <span class="text-[10px] font-bold uppercase tracking-wider {penalized ? 'text-red-700' : 'text-green-700'}">Copy/paste check</span>
      <span class="min-w-0 flex-1 text-[12px] {penalized ? 'text-red-800' : 'text-green-800'}">
        {similarityAdjustment.note.charAt(0).toUpperCase() + similarityAdjustment.note.slice(1)}{#if topSrc}
          &nbsp;<a href="/s/{topSrc.join_code}" class="font-semibold underline">open {topSrc.join_code}</a>{/if}
      </span>
      {#if penalized}
        <span class="shrink-0 text-[14px] font-bold tabular-nums text-red-700">
          {similarityAdjustment.point_delta} pts
        </span>
      {/if}
    </div>
  {/if}

  {#if summary.judges.length > 0}
    <div class="mt-4">
      <div class="mb-1.5 flex items-baseline justify-between gap-2">
        <span class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Judges</span>
        <span class="flex items-baseline gap-2">
          <span class="text-[11px] text-brand-muted">
            {summary.verdictCount}
            {summary.verdictCount === 1 ? 'evaluation' : 'evaluations'}
          </span>
          <span class="text-[11px] font-bold tabular-nums {summary.judgePoints >= 0 ? 'text-green-600' : 'text-brand-red'}">{pointsChip(summary.judgePoints)} pts</span>
        </span>
      </div>
      <div class="flex flex-wrap gap-1.5">
        {#each summary.judges as j (j.slug)}
          <HoverCard.Root openDelay={150}>
            <HoverCard.Trigger
              class="inline-flex cursor-default items-center gap-1.5 rounded-full border border-brand-border/60 bg-brand-light-blue/60 py-0.5 pl-1 pr-2 no-underline"
              data-testid="{testid}-judge-{j.slug}"
            >
              {#if judgeAvatars[j.slug]}
                <img src={ikAvatar(judgeAvatars[j.slug], 18)} alt="" class="h-[18px] w-[18px] rounded-full object-cover" />
              {:else}
                <span class="inline-flex h-[18px] w-[18px] items-center justify-center rounded-full bg-white text-[10px]" aria-hidden="true">⚖️</span>
              {/if}
              <span class="text-[12px] font-medium text-brand-text">{j.name}</span>
              <span class="text-[11px] font-bold tabular-nums {j.points > 0 ? 'text-green-600' : j.points < 0 ? 'text-brand-red' : 'text-brand-muted'}">{pointsChip(j.points)}</span>
              <span class="text-[10px] tabular-nums text-brand-muted">×{j.perTask.length}</span>
            </HoverCard.Trigger>
            <HoverCard.Content class="w-56" align="start">
              <p class="mb-1 text-[12px] font-semibold text-brand-text">
                {j.name}
                <span class="ml-1 font-normal text-brand-muted">
                  · {j.perTask.length}
                  {j.perTask.length === 1 ? 'evaluation' : 'evaluations'}
                </span>
              </p>
              <ul class="space-y-0.5">
                {#each j.perTask as t (t.ordinal)}
                  <li class="flex justify-between text-[12px] text-brand-text/80">
                    <span>Task #{t.ordinal}</span>
                    <span class="font-bold tabular-nums {t.points > 0 ? 'text-green-600' : t.points < 0 ? 'text-brand-red' : 'text-brand-muted'}">{pointsChip(t.points)}</span>
                  </li>
                {/each}
              </ul>
            </HoverCard.Content>
          </HoverCard.Root>
        {/each}
      </div>
    </div>
  {/if}

  {#if summary.criteria.length > 0}
    <div class="mt-4">
      <span class="mb-1.5 block text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Criteria (average)</span>
      <div class="flex flex-wrap gap-1.5">
        {#each summary.criteria as c (c.key)}
          <span class="inline-flex items-center gap-1 rounded-full border border-brand-border/60 bg-white px-2 py-0.5">
            <span class="text-[11px] text-brand-text/80">{c.title}</span>
            <span class="text-[11px] font-bold tabular-nums {c.avg >= 7 ? 'text-green-600' : c.avg >= 4 ? 'text-amber-600' : 'text-brand-red'}">{c.avg.toFixed(1)}</span>
          </span>
        {/each}
      </div>
    </div>
  {/if}
</div>
