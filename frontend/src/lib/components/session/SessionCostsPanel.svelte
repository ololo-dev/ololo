<script lang="ts">
  import { browser } from '$app/environment';
  import { getSettings, getSessionCosts, type SessionCosts } from '$lib/api';

  let { sessionId, isAdmin = false }: { sessionId: string; isAdmin?: boolean } = $props();

  // Admin-only, and only when the settings switch is on. Both checks happen
  // client-side against admin endpoints — non-admins get a 403 and see
  // nothing, which is exactly the contract.
  let costs = $state<SessionCosts | null>(null);

  $effect(() => {
    if (!browser || !isAdmin || !sessionId) return;
    let cancelled = false;
    (async () => {
      try {
        const settings = await getSettings();
        if ((settings as Record<string, string>)['show_llm_costs_in_session'] !== 'true') return;
        const data = await getSessionCosts(sessionId);
        if (!cancelled) costs = data;
      } catch {
        // Not an admin, endpoint missing, or nothing recorded — stay silent.
      }
    })();
    return () => {
      cancelled = true;
    };
  });

  function usd(v: number | null): string {
    if (v == null) return '—';
    if (v >= 1) return `$${v.toFixed(2)}`;
    if (v >= 0.01) return `$${v.toFixed(3)}`;
    return v > 0 ? `$${v.toFixed(5)}` : '$0';
  }
</script>

{#if costs && costs.totals.requests > 0}
  <div class="rounded-[8px] bg-white px-[24px] py-[20px]" data-testid="session-costs">
    <div class="mb-[12px] flex items-baseline justify-between">
      <h2 class="font-heading text-[16px] font-bold" style="color: #363636;">
        LLM costs
        <span class="ml-2 rounded-full bg-brand-blue/10 px-2 py-0.5 text-[11px] font-semibold text-brand-blue">admin</span>
      </h2>
      <span class="text-[15px] font-bold tabular-nums" style="color: #363636;">
        {usd(costs.totals.cost)}
        {#if costs.totals.unpriced_requests > 0}<span class="text-amber-500" title="{costs.totals.unpriced_requests} requests use unpriced models">*</span>{/if}
      </span>
    </div>
    <div class="grid gap-x-8 gap-y-1 sm:grid-cols-2">
      <div>
        <p class="mb-1 text-[11px] font-semibold uppercase tracking-wider text-brand-muted">By player</p>
        {#each costs.by_player as b (b.key)}
          <div class="flex items-baseline justify-between text-[13px]">
            <span style="color: #363636;">{b.label ?? b.key}</span>
            <span class="tabular-nums font-medium" style="color: #363636;">{usd(b.cost)}</span>
          </div>
        {/each}
      </div>
      <div>
        <p class="mb-1 text-[11px] font-semibold uppercase tracking-wider text-brand-muted">By judge</p>
        {#each costs.by_judge as b (b.key)}
          <div class="flex items-baseline justify-between text-[13px]">
            <span style="color: #363636;">{b.key}</span>
            <span class="tabular-nums font-medium" style="color: #363636;">{usd(b.cost)}</span>
          </div>
        {/each}
      </div>
    </div>
  </div>
{/if}
