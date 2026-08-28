<script lang="ts">
  import type { PlayerTaskSummaryEntry, TaskStatsEntry } from '$lib/types/arena';
  import { formatTokens } from '$lib/format';

  type Props = {
    /** Tasks in display order; only revealed tasks should be passed in. */
    tasks: PlayerTaskSummaryEntry[];
    statsByOrdinal: Map<number, TaskStatsEntry>;
  };
  let { tasks, statsByOrdinal }: Props = $props();

  type Row = { task: PlayerTaskSummaryEntry | null; ordinal: number; stats: TaskStatsEntry };

  // One row per reported stats entry, ordered by task ordinal. Entries whose
  // project task was deleted still carry the ordinal, so they stay visible.
  const rows = $derived.by<Row[]>(() => {
    const byOrdinal = new Map(tasks.map((t) => [t.ordinal, t]));
    return [...statsByOrdinal.values()]
      .sort((a, b) => a.task_ordinal - b.task_ordinal)
      .map((s) => ({
        task: byOrdinal.get(s.task_ordinal) ?? null,
        ordinal: s.task_ordinal,
        stats: s,
      }));
  });

  function windowSecs(s: TaskStatsEntry): number | null {
    if (!s.window_started_at || !s.window_ended_at) return null;
    const ms = new Date(s.window_ended_at).getTime() - new Date(s.window_started_at).getTime();
    return ms >= 0 ? Math.round(ms / 1000) : null;
  }

  function fmtMins(secs: number): string {
    return `${Math.floor(secs / 60)}m ${secs % 60}s`;
  }

  const totals = $derived.by(() => {
    let input = 0, output = 0, cacheRead = 0, cacheWrite = 0;
    let userMsgs = 0, agentMsgs = 0, toolCalls = 0, cost = 0, secs = 0;
    let hasCost = false;
    for (const { stats } of rows) {
      input += stats.input_tokens;
      output += stats.output_tokens;
      cacheRead += stats.cache_read_tokens;
      cacheWrite += stats.cache_write_tokens;
      userMsgs += stats.user_messages;
      agentMsgs += stats.assistant_messages;
      toolCalls += stats.tool_calls;
      if (stats.cost != null) {
        cost += stats.cost;
        hasCost = true;
      }
      secs += windowSecs(stats) ?? 0;
    }
    return { input, output, cacheRead, cacheWrite, userMsgs, agentMsgs, toolCalls, cost, hasCost, secs };
  });

  // Points come from the task summaries (probes + bonus + judges), not the
  // agent telemetry — they are the score side of the same per-task story.
  const totalPoints = $derived(
    tasks.reduce((sum, t) => sum + (t.total_points ?? 0), 0),
  );

  // Distinct agent/model pairs seen across the whole session.
  const agentModels = $derived.by(() => {
    const map = new Map<string, { agent: string; model: string | null; sessions: number }>();
    for (const { stats } of rows) {
      for (const a of stats.agents) {
        const key = `${a.agent}::${a.model ?? ''}`;
        const prev = map.get(key);
        if (prev) prev.sessions += 1;
        else map.set(key, { agent: a.agent, model: a.model ?? null, sessions: 1 });
      }
    }
    return [...map.values()];
  });

  // Tool usage rolled up across every agent session of every task.
  const toolTotals = $derived.by(() => {
    const map = new Map<string, number>();
    for (const { stats } of rows) {
      for (const a of stats.agents) {
        for (const [tool, n] of Object.entries(a.tools)) {
          map.set(tool, (map.get(tool) ?? 0) + n);
        }
      }
    }
    return [...map.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  });

  const skillTotals = $derived.by(() => {
    const map = new Map<string, number>();
    for (const { stats } of rows) {
      for (const a of stats.agents) {
        for (const [skill, n] of Object.entries(a.skills)) {
          map.set(skill, (map.get(skill) ?? 0) + n);
        }
      }
    }
    return [...map.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  });
</script>

<div data-testid="player-stats-tab">
  {#if rows.length === 0}
    <div class="rounded-[8px] bg-white py-16 text-center text-sm text-brand-muted shadow-sm">
      No implementation statistics reported yet. ololo uploads them after each task.
    </div>
  {:else}
    <!-- Overall -->
    <div class="mb-4 overflow-hidden rounded-[8px] bg-white p-4 shadow-sm">
      <h3 class="mb-3 font-heading text-[14px] font-semibold text-brand-text">Overall</h3>
      <div class="flex flex-wrap gap-2">
        <div class="rounded-md border border-brand-border/40 bg-brand-light-blue/10 px-3 py-2" title="Everything the model was fed, cache included — with prompt caching most of the input is a cache read, and the uncached remainder alone would be misleadingly small">
          <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Tokens in / out</div>
          <div data-testid="stats-total-tokens" class="text-[13px] font-semibold tabular-nums text-brand-text">{formatTokens(totals.input + totals.cacheRead + totals.cacheWrite)} / {formatTokens(totals.output)}</div>
        </div>
        <div class="rounded-md border border-brand-border/40 bg-brand-light-blue/10 px-3 py-2">
          <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Cache read / write</div>
          <div class="text-[13px] font-semibold tabular-nums text-brand-text">{formatTokens(totals.cacheRead)} / {formatTokens(totals.cacheWrite)}</div>
        </div>
        <div class="rounded-md border border-brand-border/40 bg-brand-light-blue/10 px-3 py-2">
          <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Messages (user / agent)</div>
          <div class="text-[13px] font-semibold tabular-nums text-brand-text">{totals.userMsgs} / {totals.agentMsgs}</div>
        </div>
        <div class="rounded-md border border-brand-border/40 bg-brand-light-blue/10 px-3 py-2">
          <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Tool calls</div>
          <div class="text-[13px] font-semibold tabular-nums text-brand-text">{totals.toolCalls}</div>
        </div>
        {#if totals.hasCost && totals.cost > 0}
          <div class="rounded-md border border-brand-border/40 bg-brand-light-blue/10 px-3 py-2">
            <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Cost</div>
            <div class="text-[13px] font-semibold tabular-nums text-brand-text">${totals.cost.toFixed(4)}</div>
          </div>
        {/if}
        {#if totals.secs > 0}
          <div class="rounded-md border border-brand-border/40 bg-brand-light-blue/10 px-3 py-2">
            <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Implementation time</div>
            <div class="text-[13px] font-semibold tabular-nums text-brand-text">{fmtMins(totals.secs)}</div>
          </div>
        {/if}
        <div class="rounded-md border border-brand-border/40 bg-brand-light-blue/10 px-3 py-2">
          <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Points</div>
          <div class="text-[13px] font-semibold tabular-nums {totalPoints >= 0 ? 'text-green-600' : 'text-red-600'}">{totalPoints >= 0 ? '+' : ''}{totalPoints}</div>
        </div>
      </div>

      {#if agentModels.length > 0}
        <div class="mt-3 flex flex-wrap items-center gap-1.5">
          <span class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Agents</span>
          {#each agentModels as am (am.agent + ':' + am.model)}
            <span class="rounded-full bg-brand-blue/10 px-2 py-0.5 text-[11px] font-semibold text-brand-blue">
              {am.agent}{#if am.model}&nbsp;· {am.model}{/if}
            </span>
          {/each}
        </div>
      {/if}
    </div>

    <!-- Per-task breakdown -->
    <div class="mb-4 overflow-hidden rounded-[8px] bg-white shadow-sm">
      <h3 class="border-b border-brand-border/40 px-4 py-3 font-heading text-[14px] font-semibold text-brand-text">By task</h3>
      <div class="overflow-x-auto">
        <table class="w-full text-[12px]">
          <thead>
            <tr class="border-b border-brand-border/40 text-left text-[10px] uppercase tracking-wider text-brand-muted">
              <th class="px-4 py-2">Task</th>
              <th class="px-4 py-2 text-right">Tokens in / out</th>
              <th class="px-4 py-2 text-right">Cache r / w</th>
              <th class="px-4 py-2 text-right">Msgs u / a</th>
              <th class="px-4 py-2 text-right">Tools</th>
              <th class="px-4 py-2 text-right">Time</th>
              <th class="px-4 py-2 text-right">Points</th>
            </tr>
          </thead>
          <tbody>
            {#each rows as row (row.ordinal)}
              {@const secs = windowSecs(row.stats)}
              <tr class="border-b border-brand-border/20 last:border-0" data-testid="stats-row-{row.ordinal}">
                <td class="px-4 py-2">
                  <span class="font-mono text-[10px] text-brand-blue/70">#{row.ordinal}</span>
                  <span class="ml-1.5 font-semibold text-brand-text">{row.task?.title ?? 'Deleted task'}</span>
                </td>
                <td class="px-4 py-2 text-right tabular-nums text-brand-text">{formatTokens(row.stats.input_tokens + row.stats.cache_read_tokens + row.stats.cache_write_tokens)} / {formatTokens(row.stats.output_tokens)}</td>
                <td class="px-4 py-2 text-right tabular-nums text-brand-muted">{formatTokens(row.stats.cache_read_tokens)} / {formatTokens(row.stats.cache_write_tokens)}</td>
                <td class="px-4 py-2 text-right tabular-nums text-brand-muted">{row.stats.user_messages} / {row.stats.assistant_messages}</td>
                <td class="px-4 py-2 text-right tabular-nums text-brand-muted">{row.stats.tool_calls}</td>
                <td class="px-4 py-2 text-right tabular-nums text-brand-muted">{secs != null ? fmtMins(secs) : '—'}</td>
                <td class="px-4 py-2 text-right tabular-nums font-semibold {(row.task?.total_points ?? 0) >= 0 ? 'text-green-600' : 'text-red-600'}">
                  {#if row.task?.total_points != null}{row.task.total_points >= 0 ? '+' : ''}{row.task.total_points}{:else}—{/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    </div>

    <!-- Rolled-up tool + skill usage -->
    <div class="grid gap-3 sm:grid-cols-2">
      <div class="overflow-hidden rounded-[8px] bg-white p-4 shadow-sm">
        <h3 class="mb-2 font-heading text-[14px] font-semibold text-brand-text">Tools ({totals.toolCalls} calls)</h3>
        {#if toolTotals.length === 0}
          <p class="text-[12px] text-brand-muted">No tool calls recorded.</p>
        {:else}
          <ul class="space-y-0.5">
            {#each toolTotals as [tool, count] (tool)}
              <li class="flex items-center justify-between text-[12px]">
                <span class="font-mono text-brand-text">{tool}</span>
                <span class="tabular-nums text-brand-muted">{count}</span>
              </li>
            {/each}
          </ul>
        {/if}
      </div>
      <div class="overflow-hidden rounded-[8px] bg-white p-4 shadow-sm">
        <h3 class="mb-2 font-heading text-[14px] font-semibold text-brand-text">Skills loaded</h3>
        {#if skillTotals.length === 0}
          <p class="text-[12px] text-brand-muted">No skills loaded.</p>
        {:else}
          <div class="flex flex-wrap gap-1.5">
            {#each skillTotals as [skill, count] (skill)}
              <span class="rounded-full bg-brand-blue/10 px-2 py-0.5 text-[11px] font-semibold text-brand-blue">{skill}{count > 1 ? ` ×${count}` : ''}</span>
            {/each}
          </div>
        {/if}
      </div>
    </div>
  {/if}
</div>
