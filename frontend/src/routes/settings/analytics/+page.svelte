<script lang="ts">
  import {
    getCostsSummary,
    getSessionCosts,
    updateSetting,
    type CostBucket,
    type CostsSummary,
    type SessionCosts,
    type ModelPrice,
  } from '$lib/api';
  import { loadModelsDevPrices } from '$lib/models-dev';
  import { notify } from '$lib/notifications.svelte';

  let { data } = $props();

  // The page owns this state after the server load seeds it — reloads go
  // through `reload()`, not through invalidation.
  /* svelte-ignore state_referenced_locally */
  let summary = $state<CostsSummary | null>(data.summary);
  /* svelte-ignore state_referenced_locally */
  let days = $state(data.summary?.days ?? 30);
  let loading = $state(false);
  /* svelte-ignore state_referenced_locally */
  let showInSession = $state<boolean>(data.showCostsInSession);
  /* svelte-ignore state_referenced_locally */
  let prices = $state<Record<string, ModelPrice>>({ ...(data.summary?.prices ?? {}) });
  let pricesDirty = $state(false);
  let savingPrices = $state(false);
  let fillingPrices = $state(false);
  let pricesOpen = $state(false);
  // Session rows expand in place: which are open, and each one's loaded costs.
  let openSessions = $state(new Set<string>());
  let sessionCosts = $state<Record<string, SessionCosts>>({});
  let sessionLoading = $state<string | null>(null);

  async function reload(newDays: number) {
    days = newDays;
    loading = true;
    try {
      summary = await getCostsSummary(newDays);
      prices = { ...summary.prices };
      pricesDirty = false;
    } catch {
      notify.error('Failed to load cost analytics.');
    } finally {
      loading = false;
    }
  }

  async function toggleShowInSession() {
    const next = !showInSession;
    try {
      await updateSetting('show_llm_costs_in_session', String(next));
      showInSession = next;
      notify.success(next ? 'Costs will show on session pages (admins only).' : 'In-session costs hidden.');
    } catch {
      notify.error('Failed to update the setting.');
    }
  }

  /** Models that appeared in telemetry: priced first, then unpriced. */
  const priceRows = $derived.by(() => {
    const seen = new Set<string>();
    const rows: string[] = [];
    for (const b of summary?.by_model ?? []) {
      if (!seen.has(b.key)) {
        seen.add(b.key);
        rows.push(b.key);
      }
    }
    for (const m of Object.keys(prices)) {
      if (!seen.has(m)) {
        seen.add(m);
        rows.push(m);
      }
    }
    return rows;
  });

  function priceOf(model: string): ModelPrice {
    return prices[model] ?? { input: 0, output: 0, cache_read: 0, cache_write: 0 };
  }

  function setPrice(model: string, field: keyof ModelPrice, raw: string) {
    const v = Number(raw);
    if (!Number.isFinite(v) || v < 0) return;
    prices = { ...prices, [model]: { ...priceOf(model), [field]: v } };
    pricesDirty = true;
  }

  async function savePrices() {
    savingPrices = true;
    try {
      // Zero-only rows are noise — store only models with at least one price.
      const compact = Object.fromEntries(
        Object.entries(prices).filter(
          ([, p]) => p.input > 0 || p.output > 0 || p.cache_read > 0 || p.cache_write > 0,
        ),
      );
      await updateSetting('llm_model_prices', JSON.stringify(compact));
      notify.success('Prices saved.');
      await reload(days);
    } catch {
      notify.error('Failed to save prices.');
    } finally {
      savingPrices = false;
    }
  }

  async function fillFromModelsDev() {
    fillingPrices = true;
    try {
      const catalog = await loadModelsDevPrices();
      let filled = 0;
      const next = { ...prices };
      for (const model of priceRows) {
        const p = next[model];
        const known = catalog[model];
        if (!known) continue;
        if (!p || (p.input === 0 && p.output === 0)) {
          next[model] = known;
          filled += 1;
        }
      }
      prices = next;
      if (filled > 0) {
        pricesDirty = true;
        notify.success(`Filled ${filled} model price(s) from models.dev — review and save.`);
      } else {
        notify.info?.('No matching prices found on models.dev.');
      }
    } catch {
      notify.error('models.dev catalog is unreachable.');
    } finally {
      fillingPrices = false;
    }
  }

  async function toggleSession(bucket: CostBucket) {
    const id = bucket.key;
    const next = new Set(openSessions);
    if (next.has(id)) {
      next.delete(id);
      openSessions = next;
      return;
    }
    next.add(id);
    openSessions = next;
    if (!sessionCosts[id]) {
      sessionLoading = id;
      try {
        sessionCosts = { ...sessionCosts, [id]: await getSessionCosts(id) };
      } catch {
        notify.error('Failed to load session costs.');
        next.delete(id);
        openSessions = new Set(next);
      } finally {
        sessionLoading = null;
      }
    }
  }

  // ── Usage heatmap (day × hour, UTC) ──────────────────────────────────
  // Sequential single-hue ramp, light→dark (validated dataviz palette).
  const HEAT_RAMP = [
    '#cde2fb', '#b7d3f6', '#9ec5f4', '#86b6ef', '#6da7ec', '#5598e7',
    '#3987e5', '#2a78d6', '#256abf', '#1c5cab', '#184f95', '#104281', '#0d366b',
  ];

  const heatDays = $derived.by(() => {
    const out: string[] = [];
    const today = new Date();
    for (let i = (summary?.days ?? 30) - 1; i >= 0; i--) {
      const d = new Date(today.getTime() - i * 86_400_000);
      out.push(d.toISOString().slice(0, 10));
    }
    return out;
  });

  const heatCells = $derived.by(() => {
    const map = new Map<string, { requests: number; tokens: number; cost: number | null }>();
    for (const c of summary?.heatmap ?? []) map.set(`${c.day}:${c.hour}`, c);
    return map;
  });

  const heatMax = $derived.by(() => {
    let max = 0;
    for (const c of heatCells.values()) if (c.tokens > max) max = c.tokens;
    return max;
  });

  /** Per-day totals for the bar strip above the heatmap. */
  const dayTotals = $derived.by(() => {
    const map = new Map<string, { requests: number; tokens: number; cost: number }>();
    for (const c of summary?.heatmap ?? []) {
      const t = map.get(c.day) ?? { requests: 0, tokens: 0, cost: 0 };
      t.requests += c.requests;
      t.tokens += c.tokens;
      t.cost += c.cost ?? 0;
      map.set(c.day, t);
    }
    return map;
  });

  const dayMax = $derived.by(() => {
    let max = 0;
    for (const t of dayTotals.values()) if (t.tokens > max) max = t.tokens;
    return max;
  });

  function dayTitle(day: string): string {
    const t = dayTotals.get(day);
    if (!t) return `${day} — no usage`;
    const cost = t.cost > 0 ? `, ${usd(t.cost)}` : '';
    return `${day} — ${t.requests} request${t.requests === 1 ? '' : 's'}, ${tokens(t.tokens)} tokens${cost}`;
  }

  /** Cell fill: neutral for no data, then sqrt-scaled into the ramp — usage
   *  is heavy-tailed and a linear scale would wash out every ordinary hour. */
  function heatColor(tokensUsed: number): string {
    if (tokensUsed <= 0 || heatMax <= 0) return '#f0efec';
    const t = Math.sqrt(tokensUsed / heatMax);
    const idx = Math.min(HEAT_RAMP.length - 1, Math.floor(t * HEAT_RAMP.length));
    return HEAT_RAMP[idx];
  }

  function heatTitle(day: string, hour: number): string {
    const c = heatCells.get(`${day}:${hour}`);
    const at = `${day} ${String(hour).padStart(2, '0')}:00 UTC`;
    if (!c) return `${at} — no usage`;
    const cost = c.cost != null ? `, ${usd(c.cost)}` : '';
    return `${at} — ${c.requests} request${c.requests === 1 ? '' : 's'}, ${tokens(c.tokens)} tokens${cost}`;
  }

  /** Sparse day labels: all at 7d, every 5th at 30d, every Monday beyond. */
  function heatDayLabel(day: string, i: number): string {
    const d = new Date(`${day}T00:00:00Z`);
    const short = `${d.getUTCDate()} ${d.toLocaleString('en', { month: 'short', timeZone: 'UTC' })}`;
    if (heatDays.length <= 7) return short;
    if (heatDays.length <= 31) return i % 5 === 0 ? short : '';
    return d.getUTCDay() === 1 ? short : '';
  }

  function usd(v: number | null): string {
    if (v == null) return '—';
    if (v >= 1) return `$${v.toFixed(2)}`;
    if (v >= 0.01) return `$${v.toFixed(3)}`;
    return v > 0 ? `$${v.toFixed(5)}` : '$0';
  }

  function tokens(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
    return String(n);
  }
</script>

<svelte:head><title>Costs — settings</title></svelte:head>

<div class="space-y-8">
  <div class="flex flex-wrap items-center justify-between gap-3">
    <div>
      <h2 class="font-heading text-[20px] font-semibold text-brand-text">LLM costs</h2>
      <p class="mt-0.5 text-sm text-brand-muted">
        Platform spending on judges, memory, and AI operations — computed from telemetry
        tokens and the price list below.
      </p>
    </div>
    <div class="flex items-center gap-2" data-testid="days-switch">
      {#each [7, 30, 90] as d (d)}
        <button
          type="button"
          onclick={() => reload(d)}
          class="rounded-btn px-3 py-1.5 text-sm font-semibold transition-colors
                 {days === d ? 'bg-brand-blue text-white' : 'text-brand-blue hover:bg-brand-blue/10'}"
        >{d}d</button>
      {/each}
    </div>
  </div>

  {#if !summary}
    <p class="text-sm text-brand-muted">Cost analytics is unavailable (endpoint not deployed?).</p>
  {:else}
    <!-- Totals -->
    <div class="grid grid-cols-2 gap-4 sm:grid-cols-4" data-testid="totals">
      <div class="rounded-[8px] bg-white p-5 shadow-sm">
        <p class="text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Total cost</p>
        <p class="mt-1 text-2xl font-bold text-brand-text">{usd(summary.totals.cost)}</p>
        {#if summary.totals.unpriced_requests > 0}
          <p class="mt-0.5 text-[11px] text-amber-600">+{summary.totals.unpriced_requests} unpriced requests</p>
        {/if}
      </div>
      <div class="rounded-[8px] bg-white p-5 shadow-sm">
        <p class="text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Requests</p>
        <p class="mt-1 text-2xl font-bold text-brand-text">{summary.totals.requests}</p>
        {#if summary.totals.failed_requests > 0}
          <p class="mt-0.5 text-[11px] text-red-500">{summary.totals.failed_requests} failed</p>
        {/if}
      </div>
      <div class="rounded-[8px] bg-white p-5 shadow-sm">
        <p class="text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Tokens in</p>
        <p class="mt-1 text-2xl font-bold text-brand-text">{tokens(summary.totals.tokens_input)}</p>
        {#if summary.totals.tokens_cache_read > 0}
          <p class="mt-0.5 text-[11px] text-brand-muted">+{tokens(summary.totals.tokens_cache_read)} cache</p>
        {/if}
      </div>
      <div class="rounded-[8px] bg-white p-5 shadow-sm">
        <p class="text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Tokens out</p>
        <p class="mt-1 text-2xl font-bold text-brand-text">{tokens(summary.totals.tokens_output)}</p>
      </div>
    </div>

    <!-- Usage heatmap -->
    {#if (summary.heatmap ?? []).length > 0}
      <section class="rounded-[8px] bg-white p-5 shadow-sm" data-testid="usage-heatmap">
        <div class="flex flex-wrap items-baseline justify-between gap-2">
          <h3 class="text-[15px] font-bold text-brand-text">Usage by hour</h3>
          <div class="flex items-center gap-1.5 text-[11px] text-brand-muted">
            <span>fewer</span>
            {#each [0, 3, 6, 9, 12] as i (i)}
              <span class="h-3 w-3 rounded-[3px]" style="background:{HEAT_RAMP[i]}"></span>
            {/each}
            <span>more tokens · UTC</span>
          </div>
        </div>
        <!-- One shared grid: a daily-totals bar strip on top, the day × hour
             cells under it, sparse day labels at the bottom. Fluid columns
             (minmax(0,1fr)) make it fill the card at every window and day
             range — no horizontal scrollbar, ever. -->
        <div class="mt-4 grid gap-[2px]"
             style="grid-template-columns: 2.6rem repeat({heatDays.length}, minmax(0, 1fr)); grid-template-rows: 44px 8px repeat(24, 10px) 16px;">
          <div class="self-end pb-0.5 pr-1.5 text-right text-[9px] leading-none text-brand-muted"
               style="grid-row: 1; grid-column: 1;">
            {tokens(dayMax)}
          </div>
          {#each heatDays as day, di (day)}
            {@const t = dayTotals.get(day)}
            <div
              class="flex items-end"
              style="grid-row: 1; grid-column: {di + 2};"
              title={dayTitle(day)}
              role="img"
              aria-label={dayTitle(day)}
            >
              {#if t && t.tokens > 0 && dayMax > 0}
                <div
                  class="w-full rounded-t-[3px]"
                  style="height: max(2px, {Math.round((t.tokens / dayMax) * 100)}%); background: {HEAT_RAMP[6]};"
                ></div>
              {/if}
            </div>
          {/each}

          {#each Array.from({ length: 24 }, (_, h) => h) as hour (hour)}
            <div class="pr-1.5 text-right text-[9px] leading-[10px] text-brand-muted" style="grid-row: {hour + 3}; grid-column: 1;">
              {hour % 6 === 0 ? `${String(hour).padStart(2, '0')}:00` : ''}
            </div>
            {#each heatDays as day, di (day)}
              {@const c = heatCells.get(`${day}:${hour}`)}
              <div
                class="rounded-[2px] hover:outline hover:outline-2 hover:outline-brand-blue"
                style="grid-row: {hour + 3}; grid-column: {di + 2}; background:{heatColor(c?.tokens ?? 0)};"
                title={heatTitle(day, hour)}
                role="img"
                aria-label={heatTitle(day, hour)}
              ></div>
            {/each}
          {/each}

          <div style="grid-row: 27; grid-column: 1;"></div>
          {#each heatDays as day, di (day)}
            <div class="overflow-visible whitespace-nowrap text-[9px] text-brand-muted" style="grid-row: 27; grid-column: {di + 2};">
              {heatDayLabel(day, di)}
            </div>
          {/each}
        </div>
      </section>
    {/if}

    <!-- In-session switch -->
    <div class="flex items-center justify-between rounded-[8px] bg-white p-5 shadow-sm">
      <div>
        <p class="text-sm font-semibold text-brand-text">Show costs on session pages</p>
        <p class="mt-0.5 text-[13px] text-brand-muted">
          Admins see a spending panel (per player, per judge) right on the session page.
          Regular viewers never see costs.
        </p>
      </div>
      <button
        type="button"
        role="switch"
        aria-checked={showInSession}
        aria-label="Show costs on session pages"
        data-testid="show-in-session-switch"
        onclick={toggleShowInSession}
        class="relative h-6 w-11 rounded-full transition-colors {showInSession ? 'bg-brand-blue' : 'bg-gray-300'}"
      >
        <span
          class="absolute top-0.5 h-5 w-5 rounded-full bg-white shadow transition-all {showInSession ? 'left-[22px]' : 'left-0.5'}"
        ></span>
      </button>
    </div>

    <!-- Prices -->
    <section class="rounded-[8px] bg-white p-5 shadow-sm">
      <div class="flex flex-wrap items-center justify-between gap-2">
        <button
          type="button"
          class="flex items-center gap-2 text-left"
          aria-expanded={pricesOpen}
          data-testid="prices-toggle"
          onclick={() => (pricesOpen = !pricesOpen)}
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" aria-hidden="true"
               class="shrink-0 text-brand-muted transition-transform {pricesOpen ? 'rotate-90' : ''}">
            <path d="M9 18l6-6-6-6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
          </svg>
          <div>
            <h3 class="text-[15px] font-bold text-brand-text">Model prices</h3>
            <p class="mt-0.5 text-[13px] text-brand-muted">
              USD per million tokens · {priceRows.length} model{priceRows.length === 1 ? '' : 's'} seen in telemetry.
            </p>
          </div>
        </button>
        {#if pricesOpen}
          <div class="flex gap-2">
            <button
              type="button"
              onclick={fillFromModelsDev}
              disabled={fillingPrices}
              class="rounded-btn border border-brand-blue px-3 py-1.5 text-sm font-semibold text-brand-blue transition-colors hover:bg-brand-blue/10 disabled:opacity-40"
            >{fillingPrices ? 'Fetching…' : 'Fill from models.dev'}</button>
            <button
              type="button"
              onclick={savePrices}
              disabled={!pricesDirty || savingPrices}
              class="rounded-btn bg-brand-blue px-3 py-1.5 text-sm font-semibold text-white transition-opacity hover:opacity-80 disabled:opacity-40"
            >{savingPrices ? 'Saving…' : 'Save prices'}</button>
          </div>
        {/if}
      </div>
      {#if summary.unpriced_models.length > 0}
        <p class="mt-2 text-[12px] text-amber-600">
          No price for: {summary.unpriced_models.join(', ')} — their cost is not counted.
        </p>
      {/if}
      {#if pricesOpen}
        <div class="mt-3 overflow-x-auto">
          <table class="w-full text-sm">
            <thead>
              <tr class="border-b border-brand-border text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
                <th class="py-2 pr-4">Model</th>
                <th class="py-2 pr-4">Input</th>
                <th class="py-2 pr-4">Output</th>
                <th class="py-2 pr-4">Cache read</th>
                <th class="py-2 pr-4">Cache write</th>
              </tr>
            </thead>
            <tbody>
              {#each priceRows as model (model)}
                {@const p = priceOf(model)}
                <tr class="border-b border-brand-border/50 last:border-0">
                  <td class="py-2 pr-4 font-mono text-[13px]">{model}</td>
                  {#each ['input', 'output', 'cache_read', 'cache_write'] as f (f)}
                    <td class="py-1 pr-4">
                      <input
                        type="number"
                        min="0"
                        step="0.01"
                        value={p[f as keyof ModelPrice]}
                        oninput={(e) => setPrice(model, f as keyof ModelPrice, e.currentTarget.value)}
                        class="w-24 rounded border border-brand-border px-2 py-1 text-[13px] focus:border-brand-blue focus:outline-none"
                      />
                    </td>
                  {/each}
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </section>

    <!-- Breakdown tables -->
    {#snippet bucketHead()}
      <thead>
        <tr class="border-b border-brand-border text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
          <th class="py-2 pr-4">Name</th>
          <th class="py-2 pr-4 text-right">Requests</th>
          <th class="py-2 pr-4 text-right">Tokens in</th>
          <th class="py-2 pr-4 text-right">Tokens out</th>
          <th class="py-2 pr-0 text-right">Cost</th>
        </tr>
      </thead>
    {/snippet}

    {#snippet bucketCells(b: CostBucket)}
      <td class="py-2 pr-4 text-right tabular-nums">{b.requests}{#if b.failed_requests > 0}<span class="text-red-400"> ({b.failed_requests}✗)</span>{/if}</td>
      <td class="py-2 pr-4 text-right tabular-nums">{tokens(b.tokens_input)}</td>
      <td class="py-2 pr-4 text-right tabular-nums">{tokens(b.tokens_output)}</td>
      <td class="py-2 pr-0 text-right font-semibold tabular-nums">{usd(b.cost)}{#if b.unpriced_requests > 0}<span class="text-amber-500">*</span>{/if}</td>
    {/snippet}

    {#snippet bucketTable(title: string, rows: CostBucket[], testid: string)}
      <section class="rounded-[8px] bg-white p-5 shadow-sm">
        <h3 class="text-[15px] font-bold text-brand-text">{title}</h3>
        {#if rows.length === 0}
          <p class="mt-2 text-[13px] text-brand-muted">Nothing in this window.</p>
        {:else}
          <div class="mt-3 overflow-x-auto">
            <table class="w-full text-sm" data-testid={testid}>
              {@render bucketHead()}
              <tbody>
                {#each rows as b (b.key)}
                  <tr class="border-b border-brand-border/50 last:border-0">
                    <td class="py-2 pr-4">
                      <span class="font-medium text-brand-text">{b.label ?? b.key}</span>
                    </td>
                    {@render bucketCells(b)}
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {/if}
      </section>
    {/snippet}

    <div class="grid gap-6 lg:grid-cols-2">
      {@render bucketTable('By judge', summary.by_judge, 'by-judge')}
      {@render bucketTable('By operation', summary.by_operation, 'by-operation')}
    </div>
    {@render bucketTable('By model', summary.by_model, 'by-model')}
    {@render bucketTable('By player — all sessions', summary.by_player ?? [], 'by-player')}

    <!-- By session: each row expands into its player breakdown in place. -->
    <section class="rounded-[8px] bg-white p-5 shadow-sm">
      <h3 class="text-[15px] font-bold text-brand-text">By session — click a row for its players</h3>
      {#if summary.by_session.length === 0}
        <p class="mt-2 text-[13px] text-brand-muted">Nothing in this window.</p>
      {:else}
        <div class="mt-3 overflow-x-auto">
          <table class="w-full text-sm" data-testid="by-session">
            {@render bucketHead()}
            <tbody>
              {#each summary.by_session as b (b.key)}
                {@const open = openSessions.has(b.key)}
                <tr
                  class="cursor-pointer border-b border-brand-border/50 last:border-0 hover:bg-brand-light-blue/30"
                  onclick={() => toggleSession(b)}
                >
                  <td class="py-2 pr-4">
                    <span class="flex items-center gap-2">
                      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" aria-hidden="true"
                           class="shrink-0 text-brand-muted transition-transform {open ? 'rotate-90' : ''}">
                        <path d="M9 18l6-6-6-6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                      </svg>
                      <span class="font-medium text-brand-text">{b.label ?? b.key}</span>
                      {#if b.sublabel}
                        <span class="text-[11px] text-brand-muted">{b.sublabel}</span>
                      {/if}
                      {#if sessionLoading === b.key}
                        <span class="text-[11px] text-brand-muted">loading…</span>
                      {/if}
                    </span>
                  </td>
                  {@render bucketCells(b)}
                </tr>
                {#if open && sessionCosts[b.key]}
                  {@const sc = sessionCosts[b.key]}
                  <tr class="border-b border-brand-border/50 last:border-0 bg-brand-light-blue/10" data-testid="session-players-{b.key}">
                    <td colspan="5" class="py-2 pr-0 pl-6">
                      {#if sc.by_player.length === 0}
                        <p class="py-2 text-[13px] text-brand-muted">No player-attributed spending in this session.</p>
                      {:else}
                        <table class="w-full text-sm">
                          <tbody>
                            {#each sc.by_player as p (p.key)}
                              <tr class="border-b border-brand-border/30 last:border-0">
                                <td class="py-1.5 pr-4 text-[13px] text-brand-text">{p.label ?? p.key}</td>
                                {@render bucketCells(p)}
                              </tr>
                            {/each}
                          </tbody>
                        </table>
                      {/if}
                      <a class="mt-1 inline-block text-[12px] text-brand-blue hover:underline" href="/s/{sc.join_code}">open session →</a>
                    </td>
                  </tr>
                {/if}
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </section>

    {#if loading}
      <p class="text-sm text-brand-muted">Loading…</p>
    {/if}
  {/if}
</div>
