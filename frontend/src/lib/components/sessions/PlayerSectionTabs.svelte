<script lang="ts">
  import type { SectionTab, SectionTabId } from '$lib/sessions/player-tabs';

  let {
    tabs,
    active,
    onSelect,
  }: {
    tabs: SectionTab[];
    active: SectionTabId;
    onSelect: (id: SectionTabId) => void;
  } = $props();

  function handleKeydown(e: KeyboardEvent): void {
    const idx = tabs.findIndex((t) => t.id === active);
    if (idx < 0) return;
    if (e.key === 'ArrowRight') {
      e.preventDefault();
      onSelect(tabs[(idx + 1) % tabs.length].id);
    } else if (e.key === 'ArrowLeft') {
      e.preventDefault();
      onSelect(tabs[(idx - 1 + tabs.length) % tabs.length].id);
    }
  }
</script>

<div
  class="mb-4 flex gap-1 border-b border-brand-border/60"
  role="tablist"
  tabindex="-1"
  onkeydown={handleKeydown}
>
  {#each tabs as t (t.id)}
    <button
      type="button"
      data-testid="ptab-{t.id}"
      role="tab"
      aria-selected={active === t.id}
      onclick={() => onSelect(t.id)}
      class="px-3 py-2 font-heading text-[15px] font-semibold transition-colors {active ===
      t.id
        ? 'border-b-2 border-brand-blue text-brand-blue'
        : 'text-brand-muted hover:text-brand-text'}"
    >
      {t.label}{#if t.count !== null}<span
          class="ml-1.5 text-[12px] font-semibold tabular-nums opacity-70"
          >{t.count}</span
        >{/if}
    </button>
  {/each}
</div>
