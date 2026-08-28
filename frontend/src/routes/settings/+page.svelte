<script lang="ts">
  // The Overview dashboard: the instance at a glance — who is on it, what is
  // running right now, what the LLMs cost — with each tile linking into the
  // settings section that owns the detail.

  import { statusClass, statusLabels } from '$lib/session-status';

  let { data } = $props();

  const DAY_MS = 86_400_000;

  const userCount = $derived(data.users?.length ?? null);
  const newUsers30d = $derived.by(() => {
    if (!data.users) return 0;
    const cutoff = Date.now() - 30 * DAY_MS;
    return data.users.filter((u) => new Date(u.created_at).getTime() >= cutoff).length;
  });
  const liveSessions = $derived(data.runningTotal + data.lobbyTotal);

  const serversOnline = $derived(
    data.gameServers?.filter((s) => s.status === 'active').length ?? null,
  );
  const activeOnServers = $derived(
    (data.gameServers ?? []).reduce((sum, s) => sum + s.active_sessions, 0),
  );

  // ── Daily usage bars (last 30 days, from the costs heatmap) ──────────
  const usageDays = $derived.by(() => {
    const out: string[] = [];
    const today = new Date();
    for (let i = (data.costs?.days ?? 30) - 1; i >= 0; i--) {
      out.push(new Date(today.getTime() - i * DAY_MS).toISOString().slice(0, 10));
    }
    return out;
  });

  const dayTotals = $derived.by(() => {
    const map = new Map<string, { requests: number; tokens: number; cost: number }>();
    for (const c of data.costs?.heatmap ?? []) {
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

  function relativeTime(iso: string): string {
    const diffSec = Math.max(0, Math.floor((Date.now() - new Date(iso).getTime()) / 1000));
    if (diffSec < 60) return `${diffSec}s ago`;
    if (diffSec < 3600) return `${Math.floor(diffSec / 60)}m ago`;
    if (diffSec < 86400) return `${Math.floor(diffSec / 3600)}h ago`;
    return `${Math.floor(diffSec / 86400)}d ago`;
  }

  function serverStatusClass(status: string): string {
    switch (status) {
      case 'active': return 'bg-emerald-100 text-emerald-700';
      case 'draining': return 'bg-amber-100 text-amber-700';
      case 'inactive': return 'bg-red-100 text-red-700';
      default: return 'bg-gray-100 text-gray-700';
    }
  }
</script>

<svelte:head><title>Overview — settings</title></svelte:head>

<div class="flex flex-col gap-6">
  <div>
    <h2 class="font-heading text-[20px] font-semibold text-brand-text">Overview</h2>
    <p class="mt-0.5 text-sm text-brand-muted">
      The instance at a glance. Each tile opens its settings section.
    </p>
  </div>

  <!-- Stat tiles -->
  <div class="grid grid-cols-2 gap-4 lg:grid-cols-4" data-testid="overview-tiles">
    <a href="/settings/users" class="rounded-[8px] bg-white p-5 shadow-sm transition-shadow hover:shadow-md">
      <p class="text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Users</p>
      <p class="mt-1 text-2xl font-bold text-brand-text">{userCount ?? '—'}</p>
      <p class="mt-0.5 text-[11px] text-brand-muted">
        {#if newUsers30d > 0}+{newUsers30d} in 30 days{:else}&nbsp;{/if}
      </p>
    </a>
    <a href="/settings/sessions" class="rounded-[8px] bg-white p-5 shadow-sm transition-shadow hover:shadow-md">
      <p class="text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Sessions played</p>
      <p class="mt-1 text-2xl font-bold text-brand-text">{data.finishedTotal}</p>
      <p class="mt-0.5 text-[11px] text-brand-muted">
        {data.sessionTotal} total{#if liveSessions > 0} · <span class="font-semibold text-emerald-600">{liveSessions} live</span>{/if}
      </p>
    </a>
    <a href="/settings/analytics" class="rounded-[8px] bg-white p-5 shadow-sm transition-shadow hover:shadow-md">
      <p class="text-[11px] font-semibold uppercase tracking-wider text-brand-muted">LLM cost · 30d</p>
      <p class="mt-1 text-2xl font-bold text-brand-text">{usd(data.costs?.totals.cost ?? null)}</p>
      <p class="mt-0.5 text-[11px] text-brand-muted">
        {#if data.costs}{data.costs.totals.requests} requests · {tokens(data.costs.totals.tokens_output)} tok out{:else}no data{/if}
      </p>
    </a>
    <a href="/settings/game-servers" class="rounded-[8px] bg-white p-5 shadow-sm transition-shadow hover:shadow-md">
      <p class="text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Game servers</p>
      <p class="mt-1 text-2xl font-bold text-brand-text">
        {#if data.gameServers}{serversOnline}/{data.gameServers.length}{:else}—{/if}
      </p>
      <p class="mt-0.5 text-[11px] text-brand-muted">
        {#if data.gameServers}online · {activeOnServers} active session{activeOnServers === 1 ? '' : 's'}{:else}no data{/if}
      </p>
    </a>
  </div>

  <!-- Daily usage bars -->
  {#if data.costs && dayMax > 0}
    <section class="rounded-[8px] bg-white p-5 shadow-sm" data-testid="overview-usage">
      <div class="flex flex-wrap items-baseline justify-between gap-2">
        <h3 class="text-[15px] font-bold text-brand-text">LLM usage — tokens per day</h3>
        <a href="/settings/analytics" class="text-[12px] font-semibold text-brand-blue hover:underline">costs →</a>
      </div>
      <!-- Fluid columns: fills the card at every width, no scrollbar. -->
      <div class="mt-3 grid items-end gap-[2px]" style="grid-template-columns: repeat({usageDays.length}, minmax(0, 1fr)); height: 64px;">
        {#each usageDays as day (day)}
          {@const t = dayTotals.get(day)}
          <div
            class="flex h-full items-end"
            title={dayTitle(day)}
            role="img"
            aria-label={dayTitle(day)}
          >
            {#if t && t.tokens > 0}
              <div
                class="w-full rounded-t-[3px] bg-[#3987e5]"
                style="height: max(2px, {Math.round((t.tokens / dayMax) * 100)}%)"
              ></div>
            {:else}
              <div class="h-[2px] w-full rounded-[1px] bg-[#f0efec]"></div>
            {/if}
          </div>
        {/each}
      </div>
      <div class="mt-1 flex justify-between text-[10px] text-brand-muted">
        <span>{usageDays[0]}</span>
        <span>peak {tokens(dayMax)}</span>
        <span>{usageDays[usageDays.length - 1]}</span>
      </div>
    </section>
  {/if}

  <div class="grid gap-6 lg:grid-cols-2">
    <!-- Recent sessions -->
    <section class="rounded-[8px] bg-white shadow-sm" data-testid="overview-recent-sessions">
      <div class="flex items-baseline justify-between gap-2 p-5 pb-3">
        <h3 class="text-[15px] font-bold text-brand-text">Recent sessions</h3>
        <a href="/settings/sessions" class="text-[12px] font-semibold text-brand-blue hover:underline">all →</a>
      </div>
      {#if data.recentSessions.length === 0}
        <p class="px-5 pb-5 text-[13px] text-brand-muted">No sessions yet.</p>
      {:else}
        <ul class="divide-y divide-brand-border/60">
          {#each data.recentSessions as s (s.id)}
            <li class="flex items-center gap-3 px-5 py-2.5">
              <span class="flex min-w-0 flex-1 flex-col leading-tight">
                <span class="flex min-w-0 items-baseline gap-2">
                  <a href="/s/{s.join_code}" class="shrink-0 font-mono text-sm font-bold text-brand-blue hover:underline">{s.join_code}</a>
                  <span class="min-w-0 truncate text-xs text-brand-muted">{s.project_name ?? s.name}</span>
                </span>
                <span class="truncate text-[11px] text-brand-muted/80">
                  {s.owner_display_name ?? '—'} · {relativeTime(s.created_at)}
                </span>
              </span>
              <span class="shrink-0 rounded-full px-2.5 py-0.5 text-[11px] font-semibold {statusClass(s.status)}">
                {statusLabels[s.status] ?? s.status}
              </span>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <!-- Game servers -->
    <section class="rounded-[8px] bg-white shadow-sm" data-testid="overview-game-servers">
      <div class="flex items-baseline justify-between gap-2 p-5 pb-3">
        <h3 class="text-[15px] font-bold text-brand-text">Game servers</h3>
        <a href="/settings/game-servers" class="text-[12px] font-semibold text-brand-blue hover:underline">manage →</a>
      </div>
      {#if !data.gameServers || data.gameServers.length === 0}
        <p class="px-5 pb-5 text-[13px] text-brand-muted">No game servers registered.</p>
      {:else}
        <ul class="divide-y divide-brand-border/60">
          {#each data.gameServers as gs (gs.id)}
            <li class="flex items-center gap-3 px-5 py-2.5">
              <span class="flex min-w-0 flex-1 flex-col leading-tight">
                <span class="truncate text-sm font-semibold text-brand-text" title={gs.url}>
                  {gs.display_name ?? gs.url}
                </span>
                <span class="truncate text-[11px] text-brand-muted/80">
                  {gs.active_sessions}/{gs.capacity} sessions · heartbeat {relativeTime(gs.last_heartbeat)}
                </span>
              </span>
              <span class="shrink-0 rounded-full px-2.5 py-0.5 text-[11px] font-semibold {serverStatusClass(gs.status)}">
                {gs.status}
              </span>
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  </div>
</div>
