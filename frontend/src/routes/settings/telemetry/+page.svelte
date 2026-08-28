<script lang="ts">
  import { browser } from '$app/environment';
  import { untrack } from 'svelte';
  import { Drawer } from 'vaul-svelte';
  import {
    getLlmTelemetry,
    getLlmTelemetryRecord,
    type LlmTelemetryRow,
    type LlmTelemetryOperation,
    type LlmTelemetryStatus,
  } from '$lib/api';
  import { formatTokens, formatUsd } from '$lib/format';
  import ProviderIcon from '$lib/components/ProviderIcon.svelte';
  import TraceTimeline from '$lib/components/TraceTimeline.svelte';
  import type { JudgeLogEvent } from '$lib/types/arena';

  let { data } = $props();

  const LIMIT = 50;

  const OPERATIONS: { key: LlmTelemetryOperation; label: string }[] = [
    { key: 'judge', label: 'Judge' },
    { key: 'memory', label: 'Memory' },
    { key: 'adaptation', label: 'Adaptation' },
    { key: 'project_ai', label: 'Project AI' },
    { key: 'task_ai', label: 'Task AI' },
  ];

  function opLabel(op: string): string {
    return OPERATIONS.find((o) => o.key === op)?.label ?? op;
  }

  let items = $state<LlmTelemetryRow[]>(untrack(() => data.telemetry.items));
  let total = $state<number>(untrack(() => data.telemetry.total));
  let opFilter = $state<'' | LlmTelemetryOperation>('');
  let statusFilter = $state<'' | LlmTelemetryStatus>('');
  let offset = $state(0);
  let loading = $state(false);
  let loadError = $state<string | null>(null);

  async function refresh() {
    loading = true;
    loadError = null;
    try {
      const page = await getLlmTelemetry({
        operation: opFilter || undefined,
        status: statusFilter || undefined,
        limit: LIMIT,
        offset,
      });
      items = page.items;
      total = page.total;
    } catch {
      loadError = 'Could not load telemetry (is the backend up to date?).';
    } finally {
      loading = false;
    }
  }

  function applyFilters() {
    offset = 0;
    void refresh();
  }

  function prevPage() {
    if (offset === 0) return;
    offset = Math.max(0, offset - LIMIT);
    void refresh();
  }

  function nextPage() {
    if (offset + LIMIT >= total) return;
    offset = offset + LIMIT;
    void refresh();
  }

  // ---------- Detail drawer ----------

  let selected = $state<LlmTelemetryRow | null>(null);
  /** Metadata block is collapsed by default once a trace is present. */
  let requestOpen = $state(false);

  function openRecord(row: LlmTelemetryRow) {
    selected = row;
    requestOpen = false;
    // Refresh from the single-record endpoint; the list row is the fallback.
    void (async () => {
      try {
        const fresh = await getLlmTelemetryRecord(row.id);
        if (selected && selected.id === fresh.id) selected = fresh;
      } catch {
        // Keep the list row's data.
      }
    })();
  }

  /** detail_json parsed into label/value pairs; null when absent or invalid. */
  const detailFields = $derived.by(() => {
    if (!selected?.detail_json) return null;
    try {
      const parsed: unknown = JSON.parse(selected.detail_json);
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
        return Object.entries(parsed as Record<string, unknown>);
      }
    } catch {
      // Fall through to the raw display.
    }
    return null;
  });

  function isBlockValue(key: string, value: unknown): boolean {
    if (typeof value !== 'string') return typeof value === 'object' && value !== null;
    return key.includes('excerpt') || value.length > 60 || value.includes('\n');
  }

  function blockText(value: unknown): string {
    return typeof value === 'string' ? value : JSON.stringify(value, null, 2);
  }

  /** Parsed per-turn trace; null when absent or malformed. */
  const traceEvents = $derived.by<JudgeLogEvent[] | null>(() => {
    if (!selected?.events_json) return null;
    try {
      const parsed: unknown = JSON.parse(selected.events_json);
      if (!Array.isArray(parsed)) return null;
      const events = parsed.filter(
        (e): e is JudgeLogEvent =>
          !!e && typeof e === 'object' && typeof (e as JudgeLogEvent).at_ms === 'number',
      );
      return events.length > 0 ? events : null;
    } catch {
      return null;
    }
  });

  /**
   * The detail excerpts are a lossy summary of what the trace already shows
   * verbatim, so they stay collapsed behind a toggle whenever a trace exists
   * and are shown outright when it does not.
   */
  const detailsOpen = $derived(requestOpen || traceEvents === null);

  const totalTokens = $derived(
    selected
      ? selected.tokens_input +
        selected.tokens_output +
        selected.tokens_cache_read +
        selected.tokens_cache_write
      : 0,
  );

  // ---------- Formatting ----------

  /** Map registry provider ids to models.dev logo ids where they match. */
  function providerCatalogId(provider: string): string | null {
    return provider === 'ollama' || provider === 'openrouter' ? provider : null;
  }

  /**
   * What to call the endpoint a row went through.
   *
   * The registry id alone is misleading: every OpenAI-compatible provider
   * reports "custom", so three different endpoints all read the same. Prefer
   * the configured provider's own name, which older rows (written before it
   * was recorded) still lack.
   */
  function providerLabel(row: LlmTelemetryRow): string {
    return row.provider_name ?? row.provider;
  }

  function relTime(iso: string): string {
    const ms = Date.now() - new Date(iso).getTime();
    if (!Number.isFinite(ms)) return iso;
    const s = Math.max(0, Math.floor(ms / 1000));
    if (s < 60) return `${s}s ago`;
    const m = Math.floor(s / 60);
    if (m < 60) return `${m}m ago`;
    const h = Math.floor(m / 60);
    if (h < 24) return `${h}h ago`;
    const d = Math.floor(h / 24);
    if (d < 7) return `${d}d ago`;
    return new Date(iso).toLocaleDateString();
  }

  function fmtDuration(ms: number): string {
    return `${(ms / 1000).toFixed(1)}s`;
  }

  const gridCols =
    'grid grid-cols-[100px_100px_minmax(220px,1fr)_130px_70px_70px_70px] items-center gap-3 px-6';
</script>

<div class="mt-8">
  <div class="mb-4 flex flex-wrap items-end justify-between gap-3">
    <div>
      <h2 class="font-heading text-[20px] font-semibold text-brand-text">LLM Telemetry</h2>
      <p class="mt-0.5 text-sm text-brand-muted">
        {total} recorded {total === 1 ? 'call' : 'calls'}. Every server-side LLM invocation with
        tokens, timing and status.
      </p>
    </div>
    <div class="flex flex-wrap items-center gap-2">
      <select
        aria-label="Filter by operation"
        value={opFilter}
        onchange={(e) => {
          opFilter = e.currentTarget.value as '' | LlmTelemetryOperation;
          applyFilters();
        }}
        class="rounded-[6px] border border-brand-border bg-white px-2 py-1.5 text-sm text-brand-text
               focus:outline-none focus:ring-2 focus:ring-brand-blue"
      >
        <option value="">All operations</option>
        {#each OPERATIONS as op (op.key)}
          <option value={op.key}>{op.label}</option>
        {/each}
      </select>
      <select
        aria-label="Filter by status"
        value={statusFilter}
        onchange={(e) => {
          statusFilter = e.currentTarget.value as '' | LlmTelemetryStatus;
          applyFilters();
        }}
        class="rounded-[6px] border border-brand-border bg-white px-2 py-1.5 text-sm text-brand-text
               focus:outline-none focus:ring-2 focus:ring-brand-blue"
      >
        <option value="">All statuses</option>
        <option value="ok">ok</option>
        <option value="failed">failed</option>
      </select>
      <button
        type="button"
        onclick={() => void refresh()}
        disabled={loading}
        class="rounded-btn border border-brand-border px-3 py-1.5 text-sm font-semibold text-brand-text
               transition-opacity hover:opacity-80 disabled:opacity-40"
      >
        {loading ? 'Loading…' : 'Refresh'}
      </button>
    </div>
  </div>

  {#if loadError}
    <p class="mb-3 text-sm text-red-500">{loadError}</p>
  {/if}

  <div class="rounded-[8px] bg-white shadow-sm">
    {#if items.length === 0}
      <div class="flex flex-col items-center justify-center py-16 text-brand-muted">
        <p class="text-sm">
          {loading ? 'Loading…' : 'No telemetry recorded yet.'}
        </p>
      </div>
    {:else}
      <!-- Cards below `xl`, the same way the admin sessions list works: the
           settings column is ~940px wide, which seven columns do not fit. -->
      <ul class="divide-y divide-brand-border/60 xl:hidden">
        {#each items as row (row.id)}
          <li>
            <button
              type="button"
              data-testid="telemetry-card-{row.id}"
              onclick={() => openRecord(row)}
              class="flex w-full flex-col gap-2.5 p-4 text-left transition-colors hover:bg-brand-light-blue/20"
            >
              <div class="flex items-start justify-between gap-3">
                <span class="flex min-w-0 items-center gap-2 text-sm text-brand-text">
                  <ProviderIcon name={providerLabel(row)} catalogId={providerCatalogId(row.provider)} size={16} />
                  <span class="truncate">
                    <span class="text-brand-muted">{providerLabel(row)}</span> · {row.model}
                  </span>
                </span>
                <span class="shrink-0 rounded-full px-2 py-0.5 text-[10px] font-semibold
                             {row.status === 'ok' ? 'bg-green-100 text-green-700' : 'bg-red-100 text-red-600'}">
                  {row.status}
                </span>
              </div>
              <div class="flex flex-wrap items-center gap-2">
                <span class="rounded-full bg-brand-blue/10 px-2 py-0.5 text-[10px] font-semibold text-brand-blue">
                  {opLabel(row.operation)}
                </span>
                <span class="text-xs text-brand-muted" title={new Date(row.created_at).toLocaleString()}>
                  {relTime(row.created_at)}
                </span>
              </div>
              <dl class="grid grid-cols-[6rem_minmax(0,1fr)] items-center gap-x-3 gap-y-1.5 text-xs text-brand-muted">
                <dt class="font-semibold uppercase tracking-wider text-brand-muted/70">Tokens</dt>
                <dd class="tabular-nums text-brand-text">
                  {formatTokens(row.tokens_input)} / {formatTokens(row.tokens_output)}
                </dd>
                <dt class="font-semibold uppercase tracking-wider text-brand-muted/70">Cost</dt>
                <dd class="tabular-nums text-brand-text">{formatUsd(row.cost)}</dd>
                <dt class="font-semibold uppercase tracking-wider text-brand-muted/70">Duration</dt>
                <dd class="tabular-nums">{fmtDuration(row.duration_ms)}</dd>
              </dl>
            </button>
          </li>
        {/each}
      </ul>

      <!-- The seven-column grid fits the ~940px settings column at `xl`, so it
           no longer needs a horizontal scroll container. -->
      <div class="hidden xl:block">
        <div class="{gridCols} border-b border-brand-border bg-brand-light-blue/40 py-3
                    text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
          <span>Time</span>
          <span>Operation</span>
          <span>Provider · model</span>
          <span class="text-right">Tokens in / out</span>
          <span class="text-right">Cost</span>
          <span class="text-right">Duration</span>
          <span class="text-right">Status</span>
        </div>

        {#each items as row (row.id)}
          <button
            type="button"
            data-testid="telemetry-row-{row.id}"
            onclick={() => openRecord(row)}
            class="{gridCols} w-full border-b border-brand-border/60 py-3 text-left
                   transition-colors last:border-0 hover:bg-brand-light-blue/20"
          >
            <span class="text-sm text-brand-muted" title={new Date(row.created_at).toLocaleString()}>
              {relTime(row.created_at)}
            </span>
            <span>
              <span class="rounded-full bg-brand-blue/10 px-2 py-0.5 text-[10px] font-semibold text-brand-blue">
                {opLabel(row.operation)}
              </span>
            </span>
            <span class="flex min-w-0 items-center gap-2 text-sm text-brand-text">
              <ProviderIcon name={providerLabel(row)} catalogId={providerCatalogId(row.provider)} size={16} />
              <span class="truncate">
                <span class="text-brand-muted">{providerLabel(row)}</span> · {row.model}
              </span>
            </span>
            <span class="text-right text-sm tabular-nums text-brand-text">
              {formatTokens(row.tokens_input)} / {formatTokens(row.tokens_output)}
            </span>
            <span class="text-right text-sm tabular-nums text-brand-text">
              {formatUsd(row.cost)}
            </span>
            <span class="text-right text-sm tabular-nums text-brand-muted">
              {fmtDuration(row.duration_ms)}
            </span>
            <span class="text-right">
              <span class="rounded-full px-2 py-0.5 text-[10px] font-semibold
                           {row.status === 'ok' ? 'bg-green-100 text-green-700' : 'bg-red-100 text-red-600'}">
                {row.status}
              </span>
            </span>
          </button>
        {/each}
      </div>
    {/if}
  </div>

  <div class="mt-3 flex items-center justify-between">
    <p class="text-xs text-brand-muted">
      {#if total > 0}
        Showing {offset + 1}–{Math.min(offset + items.length, offset + LIMIT)} of {total}
      {:else}
        Nothing to show
      {/if}
    </p>
    <div class="flex gap-2">
      <button
        type="button"
        onclick={prevPage}
        disabled={loading || offset === 0}
        class="rounded-btn border border-brand-border px-4 py-1.5 text-sm font-semibold text-brand-text
               transition-opacity hover:opacity-80 disabled:opacity-40"
      >
        Previous
      </button>
      <button
        type="button"
        onclick={nextPage}
        disabled={loading || offset + LIMIT >= total}
        class="rounded-btn border border-brand-border px-4 py-1.5 text-sm font-semibold text-brand-text
               transition-opacity hover:opacity-80 disabled:opacity-40"
      >
        Next
      </button>
    </div>
  </div>
</div>

{#if browser}
<Drawer.Root
  open={selected !== null}
  onOpenChange={(v) => { if (!v) selected = null; }}
  direction="right"
>
  <Drawer.Portal>
    <Drawer.Overlay class="fixed inset-0 z-50 bg-black/80" />
    <Drawer.Content
      class="fixed bottom-0 right-0 top-0 z-50 flex h-full w-full max-w-full flex-col bg-white shadow-2xl sm:w-[60vw] sm:min-w-[640px] sm:max-w-[1100px]"
    >
      {#if selected}
        <div class="border-b border-brand-border px-6 py-4">
          <div class="flex items-center gap-2">
            <h2 class="font-heading text-xl font-bold text-brand-text">LLM call</h2>
            <span class="rounded-full bg-brand-blue/10 px-2 py-0.5 text-[10px] font-semibold text-brand-blue">
              {opLabel(selected.operation)}
            </span>
            <span class="rounded-full px-2 py-0.5 text-[10px] font-semibold
                         {selected.status === 'ok' ? 'bg-green-100 text-green-700' : 'bg-red-100 text-red-600'}">
              {selected.status}
            </span>
          </div>
          <p class="mt-0.5 text-sm text-brand-muted">
            {new Date(selected.created_at).toLocaleString()}
          </p>
        </div>

        <div class="flex-1 space-y-5 overflow-y-auto px-6 py-5">
          <div class="flex items-center gap-2">
            <ProviderIcon
              name={providerLabel(selected)}
              catalogId={providerCatalogId(selected.provider)}
              size={18}
            />
            <span class="text-sm font-semibold text-brand-text">{providerLabel(selected)} · {selected.model}</span>
          </div>

          <!-- Tokens + duration -->
          <div class="flex flex-wrap gap-2">
            <div class="rounded-md border border-brand-border/40 bg-brand-light-blue/10 px-3 py-2">
              <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Tokens in / out</div>
              <div class="text-[13px] font-semibold tabular-nums text-brand-text">
                {formatTokens(selected.tokens_input)} / {formatTokens(selected.tokens_output)}
              </div>
            </div>
            <div class="rounded-md border border-brand-border/40 bg-brand-light-blue/10 px-3 py-2">
              <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Cache read / write</div>
              <div class="text-[13px] font-semibold tabular-nums text-brand-text">
                {formatTokens(selected.tokens_cache_read)} / {formatTokens(selected.tokens_cache_write)}
              </div>
            </div>
            <div class="rounded-md border border-brand-border/40 bg-brand-light-blue/10 px-3 py-2">
              <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Total tokens</div>
              <div class="text-[13px] font-semibold tabular-nums text-brand-text">{formatTokens(totalTokens)}</div>
            </div>
            <div class="rounded-md border border-brand-border/40 bg-brand-light-blue/10 px-3 py-2">
              <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Duration</div>
              <div class="text-[13px] font-semibold tabular-nums text-brand-text">{fmtDuration(selected.duration_ms)}</div>
            </div>
            <div class="rounded-md border border-brand-border/40 bg-brand-light-blue/10 px-3 py-2">
              <div class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Cost</div>
              <div class="text-[13px] font-semibold tabular-nums text-brand-text">{formatUsd(selected.cost)}</div>
            </div>
          </div>

          {#if selected.error}
            <div>
              <div class="mb-1 text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Error</div>
              <pre class="max-h-48 overflow-auto whitespace-pre-wrap break-all rounded-md bg-red-50 px-3 py-2 font-mono text-[11px] text-red-700">{selected.error}</pre>
            </div>
          {/if}

          <!-- Context -->
          {#if selected.session_id || selected.player_id || selected.task_id || selected.judge_slug}
            <div>
              <div class="mb-1.5 text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Context</div>
              <div class="space-y-1.5 text-[12px]">
                {#if selected.judge_slug}
                  <div>
                    <span class="rounded-full bg-brand-blue/10 px-2 py-0.5 text-[10px] font-semibold text-brand-blue">
                      judge: {selected.judge_slug}
                    </span>
                  </div>
                {/if}
                <!-- Names, not uuids. Each falls back to the raw id only
                     when the row it pointed at is gone — telemetry outlives
                     sessions, players and tasks by design. -->
                {#if selected.session_id}
                  <div class="flex items-baseline gap-2">
                    <span class="w-16 shrink-0 text-brand-muted">Session</span>
                    {#if selected.session_code}
                      <a
                        href="/s/{selected.session_code}"
                        class="font-mono font-semibold text-brand-blue hover:underline"
                      >
                        {selected.session_code}
                      </a>
                    {:else}
                      <code class="select-all break-all font-mono text-brand-muted"
                        >{selected.session_id}</code
                      >
                    {/if}
                  </div>
                {/if}
                {#if selected.player_id}
                  <div class="flex items-baseline gap-2">
                    <span class="w-16 shrink-0 text-brand-muted">Player</span>
                    {#if selected.player_username}
                      <a
                        href="/u/{selected.player_username}"
                        class="font-semibold text-brand-blue hover:underline"
                      >
                        @{selected.player_username}
                      </a>
                    {:else if selected.player_name}
                      <!-- Anonymous player: a display name but no account to link to. -->
                      <span class="text-brand-text">{selected.player_name}</span>
                    {:else}
                      <code class="select-all break-all font-mono text-brand-muted"
                        >{selected.player_id}</code
                      >
                    {/if}
                  </div>
                {/if}
                {#if selected.task_id}
                  <div class="flex items-baseline gap-2">
                    <span class="w-16 shrink-0 text-brand-muted">Task</span>
                    {#if selected.task_title}
                      <!-- Unlinked on purpose: there is no per-task page. -->
                      <span class="text-brand-text">{selected.task_title}</span>
                    {:else}
                      <code class="select-all break-all font-mono text-brand-muted"
                        >{selected.task_id}</code
                      >
                    {/if}
                  </div>
                {/if}
              </div>
            </div>
          {/if}

          <!-- Per-turn trace -->
          {#if traceEvents}
            <TraceTimeline events={traceEvents} />
          {/if}

          <!-- Details -->
          {#if detailFields || selected.detail_json}
            {@const label = detailFields ? 'Details' : 'Details (raw)'}
            <div>
              {#if traceEvents}
                <button
                  type="button"
                  onclick={() => (requestOpen = !requestOpen)}
                  class="mb-1.5 flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wider text-brand-muted transition-colors hover:text-brand-text"
                >
                  <span>{label}</span>
                  <svg
                    width="12" height="12" viewBox="0 0 24 24" fill="none" aria-hidden="true"
                    class="transition-transform {detailsOpen ? 'rotate-180' : ''}"
                  >
                    <path d="M6 9l6 6 6-6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
                  </svg>
                </button>
              {:else}
                <div class="mb-1.5 text-[10px] font-semibold uppercase tracking-wider text-brand-muted">{label}</div>
              {/if}
              {#if detailsOpen}
                {#if detailFields}
                  <div class="space-y-2">
                    {#each detailFields as [key, value] (key)}
                      {#if isBlockValue(key, value)}
                        <div>
                          <div class="mb-0.5 text-[10px] font-semibold text-brand-muted">{key.replace(/_/g, ' ')}</div>
                          <pre class="max-h-56 overflow-auto whitespace-pre-wrap break-all rounded-md border border-brand-border/40 bg-brand-light-blue/10 px-3 py-2 font-mono text-[11px] text-brand-text">{blockText(value)}</pre>
                        </div>
                      {:else}
                        <div class="flex items-baseline gap-2 text-[12px]">
                          <span class="shrink-0 text-brand-muted">{key.replace(/_/g, ' ')}</span>
                          <span class="tabular-nums font-semibold text-brand-text">{String(value)}</span>
                        </div>
                      {/if}
                    {/each}
                  </div>
                {:else}
                  <pre class="max-h-56 overflow-auto whitespace-pre-wrap break-all rounded-md border border-brand-border/40 bg-brand-light-blue/10 px-3 py-2 font-mono text-[11px] text-brand-text">{selected.detail_json}</pre>
                {/if}
              {/if}
            </div>
          {/if}

          <p class="text-[10px] text-brand-muted">
            Record <code class="select-all font-mono">{selected.id}</code>
          </p>
        </div>

        <div class="flex justify-end border-t border-brand-border px-6 py-3">
          <button
            type="button"
            onclick={() => (selected = null)}
            class="rounded-btn border border-brand-border px-4 py-2 text-sm text-brand-text hover:bg-brand-light-blue"
          >
            Close
          </button>
        </div>
      {/if}
    </Drawer.Content>
  </Drawer.Portal>
</Drawer.Root>
{/if}
