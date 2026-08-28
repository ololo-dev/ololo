<script lang="ts">
  // Searchable model selector: trigger button + dropdown command palette with
  // models grouped by provider. Plain Svelte 5 runes + native elements
  // (no listbox lib). Free-form model ids always work — typing a value that
  // is not an exact suggestion offers a "Use '<typed>' with <provider>" row
  // per provider, so unreachable provider endpoints never block selection.

  import type { LlmProvider } from '$lib/api';
  import { providerLogoId } from '$lib/models-dev';
  import ProviderIcon from './ProviderIcon.svelte';

  type Selection = { provider_id: string; model: string };

  let {
    providers,
    value,
    onchange,
    modelsFor,
    placeholder = 'Select model…',
    label = 'Model',
    allowClear = false,
    clearLabel = 'Inherit default',
  }: {
    providers: LlmProvider[];
    value: Selection | null;
    onchange: (sel: Selection | null) => void;
    /** Suggestion loader per provider id; results should be cached by the caller. */
    modelsFor: (providerId: string) => Promise<string[]>;
    placeholder?: string;
    label?: string;
    allowClear?: boolean;
    clearLabel?: string;
  } = $props();

  const MAX_PER_PROVIDER = 50;

  let open = $state(false);
  let query = $state('');
  let activeIndex = $state(0);
  let models = $state<Record<string, string[]>>({});
  let loading = $state<Record<string, boolean>>({});

  let root = $state<HTMLElement | null>(null);
  let listEl = $state<HTMLElement | null>(null);
  let searchEl = $state<HTMLInputElement | null>(null);

  const enabledProviders = $derived(providers.filter((p) => p.enabled));

  const selectedProvider = $derived(
    value && value.provider_id ? (providers.find((x) => x.id === value.provider_id) ?? null) : null,
  );

  const triggerText = $derived.by(() => {
    if (!value || !value.provider_id || !value.model) return null;
    return `${selectedProvider?.name ?? value.provider_id} · ${value.model}`;
  });

  type Row =
    | { type: 'clear'; index: number }
    | { type: 'header'; providerId: string; name: string; catalogId: string | null; loading: boolean }
    | {
        type: 'model';
        providerId: string;
        providerName: string;
        catalogId: string | null;
        model: string;
        freeform: boolean;
        index: number;
      }
    | { type: 'note'; providerId: string; text: string };

  const rows = $derived.by(() => {
    const typed = query.trim();
    const q = typed.toLowerCase();
    const out: Row[] = [];
    let index = 0;
    if (allowClear) out.push({ type: 'clear', index: index++ });
    for (const p of enabledProviders) {
      const all = models[p.id] ?? [];
      const matches = q ? all.filter((m) => m.toLowerCase().includes(q)) : all;
      const exact = matches.some((m) => m.toLowerCase() === q);
      const isLoading = loading[p.id] === true;
      const shown = matches.slice(0, MAX_PER_PROVIDER);
      const group: Row[] = shown.map((m) => ({
        type: 'model' as const,
        providerId: p.id,
        providerName: p.name,
        catalogId: providerLogoId(p),
        model: m,
        freeform: false,
        index: 0,
      }));
      if (typed && !exact) {
        group.push({
          type: 'model',
          providerId: p.id,
          providerName: p.name,
          catalogId: providerLogoId(p),
          model: typed,
          freeform: true,
          index: 0,
        });
      }
      if (group.length === 0 && !isLoading) continue;
      out.push({
        type: 'header',
        providerId: p.id,
        name: p.name,
        catalogId: providerLogoId(p),
        loading: isLoading,
      });
      for (const r of group) {
        if (r.type === 'model') r.index = index++;
        out.push(r);
      }
      if (matches.length > shown.length) {
        out.push({
          type: 'note',
          providerId: p.id,
          text: `+ ${matches.length - shown.length} more — keep typing to narrow`,
        });
      }
    }
    return out;
  });

  const selectableCount = $derived(
    rows.filter((r) => r.type === 'clear' || r.type === 'model').length,
  );

  function openPanel() {
    open = true;
    query = '';
    activeIndex = 0;
    for (const p of enabledProviders) {
      if (p.id in models || loading[p.id]) continue;
      loading = { ...loading, [p.id]: true };
      modelsFor(p.id)
        .then((list) => {
          models = { ...models, [p.id]: list };
        })
        .catch(() => {
          models = { ...models, [p.id]: [] };
        })
        .finally(() => {
          loading = { ...loading, [p.id]: false };
        });
    }
  }

  function close() {
    open = false;
    query = '';
    activeIndex = 0;
  }

  function toggle() {
    if (open) close();
    else openPanel();
  }

  function pick(row: Row) {
    if (row.type === 'clear') onchange(null);
    else if (row.type === 'model') onchange({ provider_id: row.providerId, model: row.model });
    close();
  }

  function pickActive() {
    const row = rows.find(
      (r) => (r.type === 'clear' || r.type === 'model') && r.index === activeIndex,
    );
    if (row) pick(row);
  }

  function onSearchKeydown(e: KeyboardEvent) {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      activeIndex = Math.min(activeIndex + 1, Math.max(selectableCount - 1, 0));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      activeIndex = Math.max(activeIndex - 1, 0);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      pickActive();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      close();
    }
  }

  // Close on outside click / Escape. Effects only run in the browser, so the
  // document listeners are SSR-safe; they are attached only while open.
  $effect(() => {
    if (!open) return;
    const onDocPointerDown = (e: MouseEvent) => {
      if (root && e.target instanceof Node && !root.contains(e.target)) close();
    };
    const onDocKeydown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close();
    };
    document.addEventListener('mousedown', onDocPointerDown);
    document.addEventListener('keydown', onDocKeydown);
    return () => {
      document.removeEventListener('mousedown', onDocPointerDown);
      document.removeEventListener('keydown', onDocKeydown);
    };
  });

  // Focus the search input when the panel opens.
  $effect(() => {
    if (open) searchEl?.focus();
  });

  // Keep the active row visible while navigating with the keyboard.
  $effect(() => {
    if (!open) return;
    const el = listEl?.querySelector(`[data-idx="${activeIndex}"]`);
    el?.scrollIntoView({ block: 'nearest' });
  });
</script>

<div class="relative inline-block w-full max-w-md" bind:this={root}>
  <button
    type="button"
    aria-label={label}
    aria-expanded={open}
    onclick={toggle}
    class="flex w-full items-center justify-between gap-2 rounded-[6px] border border-brand-border
           bg-white px-3 py-2 text-left text-sm transition-colors hover:border-brand-blue/50
           focus:outline-none focus:ring-2 focus:ring-brand-blue
           {triggerText ? 'text-brand-text' : 'text-brand-muted'}"
  >
    <span class="flex min-w-0 items-center gap-2">
      {#if value && triggerText}
        <ProviderIcon
          name={selectedProvider?.name ?? value.provider_id}
          catalogId={selectedProvider ? providerLogoId(selectedProvider) : null}
          size={18}
        />
      {/if}
      <span class="truncate">{triggerText ?? placeholder}</span>
    </span>
    <svg
      class="h-4 w-4 shrink-0 text-brand-muted transition-transform {open ? 'rotate-180' : ''}"
      viewBox="0 0 20 20"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
      aria-hidden="true"
    >
      <path d="M5 7.5 10 12.5 15 7.5" stroke-linecap="round" stroke-linejoin="round" />
    </svg>
  </button>

  {#if open}
    <!-- min-w-72 keeps the palette usable on a narrow trigger, but on a phone
         that minimum can push past the screen edge — the viewport cap wins. -->
    <div
      class="absolute left-0 top-full z-30 mt-1 w-full min-w-72 max-w-[calc(100vw-2rem)]
             rounded-[8px] border border-brand-border bg-white shadow-lg"
    >
      <div class="border-b border-brand-border p-2">
        <input
          type="text"
          placeholder="Search models or type a model id…"
          aria-label="Search models"
          bind:this={searchEl}
          value={query}
          oninput={(e) => {
            query = e.currentTarget.value;
            activeIndex = 0;
          }}
          onkeydown={onSearchKeydown}
          class="w-full rounded-[6px] border border-brand-border px-3 py-1.5 text-sm text-brand-text
                 focus:outline-none focus:ring-2 focus:ring-brand-blue"
        />
      </div>

      <div class="max-h-72 overflow-y-auto py-1" bind:this={listEl}>
        {#if enabledProviders.length === 0}
          <p class="px-3 py-4 text-center text-sm text-brand-muted">
            No enabled providers. Add one in the Providers section.
          </p>
        {:else if selectableCount === 0}
          <p class="px-3 py-4 text-center text-sm text-brand-muted">
            {rows.some((r) => r.type === 'header' && r.loading)
              ? 'Loading models…'
              : 'Type a model id to use it directly.'}
          </p>
        {:else}
          {#each rows as row, i (i)}
            {#if row.type === 'clear'}
              <button
                type="button"
                data-idx={row.index}
                onclick={() => pick(row)}
                onmouseenter={() => { activeIndex = row.index; }}
                class="flex w-full items-center px-3 py-1.5 text-left text-sm italic text-brand-muted
                       {activeIndex === row.index ? 'bg-brand-light-blue/60' : ''}"
              >
                {clearLabel}
              </button>
            {:else if row.type === 'header'}
              <p class="flex items-center gap-1.5 px-3 pb-1 pt-2 text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
                <ProviderIcon name={row.name} catalogId={row.catalogId} size={16} />
                {row.name}{row.loading ? ' — loading…' : ''}
              </p>
            {:else if row.type === 'model'}
              <button
                type="button"
                data-idx={row.index}
                onclick={() => pick(row)}
                onmouseenter={() => { activeIndex = row.index; }}
                class="flex w-full items-center justify-between gap-2 px-3 py-1.5 text-left text-sm
                       text-brand-text {activeIndex === row.index ? 'bg-brand-light-blue/60' : ''}"
              >
                {#if row.freeform}
                  <span class="flex min-w-0 items-center gap-2">
                    <ProviderIcon name={row.providerName} catalogId={row.catalogId} size={16} muted />
                    <span class="truncate">
                      Use '<span class="font-medium">{row.model}</span>'
                    </span>
                  </span>
                  <span class="shrink-0 text-xs text-brand-muted">free-form</span>
                {:else}
                  <span class="flex min-w-0 items-center gap-2">
                    <ProviderIcon name={row.providerName} catalogId={row.catalogId} size={16} muted />
                    <span class="truncate">{row.model}</span>
                  </span>
                  {#if value && value.provider_id === row.providerId && value.model === row.model}
                    <svg
                      class="h-4 w-4 shrink-0 text-brand-blue"
                      viewBox="0 0 20 20"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="2"
                      aria-hidden="true"
                    >
                      <path d="m4.5 10.5 3.5 3.5 7.5-8" stroke-linecap="round" stroke-linejoin="round" />
                    </svg>
                  {/if}
                {/if}
              </button>
            {:else}
              <p class="px-3 py-1 text-xs text-brand-muted">{row.text}</p>
            {/if}
          {/each}
        {/if}
      </div>
    </div>
  {/if}
</div>
