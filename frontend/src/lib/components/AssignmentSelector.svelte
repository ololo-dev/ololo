<!--
  Picks what an LLM assignment points at: a single provider+model, or a
  whole pool.

  Wraps `ModelSelector` for the model half and adds a pool dropdown, behind
  a two-way switch. Emits the same tagged shape the API stores, so callers
  hand the value straight to `updateLlmAssignments` / `updateJudge`.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import ModelSelector from './ModelSelector.svelte';
  import { isPoolAssignment } from '$lib/api';
  import type { LlmAssignment, LlmPool, LlmProvider } from '$lib/api';

  let {
    providers,
    pools,
    value,
    onchange,
    modelsFor,
    placeholder = 'Select a model…',
    label = 'Assignment',
    clearLabel = 'Clear',
    disabled = false,
  }: {
    providers: LlmProvider[];
    pools: LlmPool[];
    value: LlmAssignment | null;
    onchange: (value: LlmAssignment | null) => void;
    modelsFor: (providerId: string) => Promise<string[]>;
    placeholder?: string;
    label?: string;
    clearLabel?: string;
    disabled?: boolean;
  } = $props();

  // Which half is on screen. Derived from the stored value, but the switch
  // sets it directly so an empty row can be pointed at a pool before
  // anything is chosen. `value` changing (a save + reload) re-syncs it.
  let kind = $state<'model' | 'pool'>(
    untrack(() => (isPoolAssignment(value) ? 'pool' : 'model')),
  );
  $effect(() => {
    kind = isPoolAssignment(value) ? 'pool' : 'model';
  });

  const poolId = $derived(isPoolAssignment(value) ? value.pool_id : '');
  const modelValue = $derived(isPoolAssignment(value) ? null : value);

  /** Switching sides drops the other side's selection rather than keeping a
   *  hidden one — only one of them is ever stored. */
  function pick(next: 'model' | 'pool') {
    if (next === kind) return;
    kind = next;
    if (value) onchange(null);
  }

  function onPoolChange(e: Event) {
    const id = (e.currentTarget as HTMLSelectElement).value;
    onchange(id ? { pool_id: id } : null);
  }

  function memberCount(p: LlmPool): string {
    const enabled = p.members.filter((m) => m.enabled).length;
    return enabled === 1 ? '1 model' : `${enabled} models`;
  }
</script>

<div class="w-full max-w-md">
  <div class="mb-1.5 inline-flex rounded-[6px] border border-brand-border p-0.5" role="group">
    <button
      type="button"
      {disabled}
      aria-pressed={kind === 'model'}
      class="rounded-[4px] px-2.5 py-1 text-[11px] font-semibold transition-colors disabled:opacity-40 {kind ===
      'model'
        ? 'bg-brand-blue text-white'
        : 'text-brand-muted hover:text-brand-text'}"
      onclick={() => pick('model')}
    >
      Model
    </button>
    <button
      type="button"
      {disabled}
      aria-pressed={kind === 'pool'}
      class="rounded-[4px] px-2.5 py-1 text-[11px] font-semibold transition-colors disabled:opacity-40 {kind ===
      'pool'
        ? 'bg-brand-blue text-white'
        : 'text-brand-muted hover:text-brand-text'}"
      onclick={() => pick('pool')}
    >
      Pool
    </button>
  </div>

  {#if kind === 'pool'}
    {#if pools.length === 0}
      <p class="text-xs text-brand-muted">
        No pools yet — create one in the Model pools section above.
      </p>
    {:else}
      <select
        {disabled}
        aria-label="{label} pool"
        value={poolId}
        onchange={onPoolChange}
        class="w-full rounded-[6px] border border-brand-border px-3 py-2 text-sm text-brand-text focus:outline-none focus:ring-2 focus:ring-brand-blue disabled:bg-gray-50"
      >
        <option value="">{clearLabel}</option>
        {#each pools as p (p.id)}
          <option value={p.id}>{p.name} · {memberCount(p)}</option>
        {/each}
      </select>
    {/if}
  {:else}
    <ModelSelector
      {providers}
      {modelsFor}
      {placeholder}
      {label}
      value={modelValue}
      allowClear={true}
      {clearLabel}
      onchange={(sel) => onchange(sel)}
    />
  {/if}
</div>
