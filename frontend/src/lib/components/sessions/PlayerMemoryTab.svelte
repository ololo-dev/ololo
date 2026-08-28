<script lang="ts">
  import type { PlayerMemoryEntry } from '$lib/types/arena';

  type Props = {
    entries: PlayerMemoryEntry[];
    /** ISO timestamp of the last extraction, or null before the first one. */
    updatedAt: string | null;
    loading: boolean;
    error: string | null;
  };
  let { entries, updatedAt, loading, error }: Props = $props();

  const extractedCount = $derived(entries.filter((e) => e.extracted).length);

  function fmtTime(iso: string): string {
    return new Date(iso).toLocaleString();
  }
</script>

<div data-testid="player-memory-tab">
  {#if error}
    <div class="rounded-[8px] bg-white py-16 text-center text-sm text-red-600 shadow-sm">
      {error}
    </div>
  {:else if entries.length === 0}
    <div class="rounded-[8px] bg-white py-16 text-center text-sm text-brand-muted shadow-sm">
      {loading ? 'Loading memory…' : 'This project does not use session memory.'}
    </div>
  {:else}
    <div class="overflow-hidden rounded-[8px] bg-white shadow-sm">
      <div class="flex items-center justify-between border-b border-brand-border/40 px-4 py-3">
        <h3 class="font-heading text-[14px] font-semibold text-brand-text">Session memory</h3>
        <div class="text-[11px] text-brand-muted">
          {#if extractedCount > 0 && updatedAt}
            Extracted from AGENTS.md / README.md · updated {fmtTime(updatedAt)}
          {:else}
            Defaults only — nothing extracted from the docs yet
          {/if}
        </div>
      </div>
      <div class="overflow-x-auto">
        <table class="w-full text-left text-[13px]">
          <thead>
            <tr class="border-b border-brand-border/40 text-[10px] font-semibold uppercase tracking-wider text-brand-muted">
              <th class="px-4 py-2">Key</th>
              <th class="px-4 py-2">Value</th>
              <th class="px-4 py-2">Default</th>
              <th class="px-4 py-2">Source</th>
            </tr>
          </thead>
          <tbody>
            {#each entries as entry (entry.key)}
              <tr class="border-b border-brand-border/20 last:border-b-0" data-testid="memory-row-{entry.key}">
                <td class="px-4 py-2 font-mono text-[12px] text-brand-text">{entry.key}</td>
                <td class="px-4 py-2 font-mono text-[12px] {entry.extracted ? 'font-semibold text-brand-text' : 'text-brand-muted'}">{entry.value}</td>
                <td class="px-4 py-2 font-mono text-[12px] text-brand-muted">{entry.default}</td>
                <td class="px-4 py-2">
                  {#if entry.extracted}
                    <span class="rounded-full bg-emerald-50 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-emerald-700">extracted</span>
                  {:else}
                    <span class="rounded-full bg-brand-light-blue/20 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-brand-muted">default</span>
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <div class="border-t border-brand-border/40 px-4 py-2 text-[11px] text-brand-muted">
        These values are re-extracted from the player's markdown files after each
        completed task and substituted into probe commands as
        <code class="font-mono">{'{memory.<key>}'}</code>.
      </div>
    </div>
  {/if}
</div>
