<script lang="ts">
  import { Plus, X } from 'lucide-svelte';

  /** One editable schema row. Empty keys are dropped on submit. */
  interface MemoryRow {
    key: string;
    value: string;
  }

  interface Props {
    open?: boolean;
    onOpenChange?: (open: boolean) => void;
    rows?: MemoryRow[];
    onchange?: (rows: MemoryRow[]) => void;
    idPrefix?: string;
  }

  let { open = false, onOpenChange, rows = [], onchange, idPrefix = '' }: Props = $props();

  // Mirrors arena-core::memory limits, which the server enforces on PATCH.
  const MAX_KEYS = 32;
  const MAX_KEY_LEN = 64;
  const MAX_VALUE_LEN = 500;

  const pid = (name: string) => (idPrefix ? `${idPrefix}-${name}` : name);

  function edit(i: number, patch: Partial<MemoryRow>) {
    onchange?.(rows.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  }

  function addRow() {
    onchange?.([...rows, { key: '', value: '' }]);
  }

  function removeRow(i: number) {
    onchange?.(rows.filter((_, j) => j !== i));
  }

  // Only rows with a key reach the server; an empty list clears the schema.
  const serialized = $derived(
    JSON.stringify(rows.filter((r) => r.key.trim().length > 0)),
  );
</script>

<div class="mb-4 mt-6 border-t border-brand-border pt-4" data-testid="memory-schema-fields">
  <button
    type="button"
    class="flex w-full items-center justify-between"
    onclick={() => onOpenChange?.(!open)}
  >
    <span class="text-xs font-semibold text-brand-text">
      Session memory
      <span class="font-normal text-brand-muted">({rows.length} keys)</span>
    </span>
    <svg
      width="16" height="16" viewBox="0 0 24 24" fill="none"
      xmlns="http://www.w3.org/2000/svg"
      class="text-brand-muted transition-transform duration-200"
      style:transform={open ? 'rotate(180deg)' : 'rotate(90deg)'}
      aria-hidden="true"
    >
      <path d="M6 9l6 6 6-6" stroke="currentColor" stroke-width="2"
        stroke-linecap="round" stroke-linejoin="round" />
    </svg>
  </button>

  {#if open}
    <p class="mt-2 text-xs text-brand-muted">
      Keys declared here are available in task command templates as
      <code class="font-mono">{'{memory.<key>}'}</code>. The value is the default; during a session
      an LLM reads the player's <code class="font-mono">AGENTS.md</code> /
      <code class="font-mono">README.md</code> and overrides it. Max {MAX_KEYS} keys; key characters:
      letters, digits, underscore.
    </p>

    {#if rows.length > 0}
      <div class="mt-3 space-y-2">
        {#each rows as row, i (i)}
          <div class="flex items-center gap-2" data-testid="memory-schema-row">
            <input
              id={pid(`memory-key-${i}`)}
              type="text"
              maxlength={MAX_KEY_LEN}
              pattern="[A-Za-z0-9_]+"
              placeholder="dev"
              aria-label="Memory key {i + 1}"
              value={row.key}
              oninput={(e) => edit(i, { key: (e.target as HTMLInputElement).value })}
              class="h-[40px] w-[220px] rounded-[8px] border-2 border-brand-border bg-white px-3
                     font-mono text-sm text-brand-text placeholder:text-brand-muted
                     focus:border-brand-blue focus:outline-none"
            />
            <input
              id={pid(`memory-value-${i}`)}
              type="text"
              maxlength={MAX_VALUE_LEN}
              placeholder="npm run dev"
              aria-label="Memory default {i + 1}"
              value={row.value}
              oninput={(e) => edit(i, { value: (e.target as HTMLInputElement).value })}
              class="h-[40px] flex-1 rounded-[8px] border-2 border-brand-border bg-white px-3
                     text-sm text-brand-text placeholder:text-brand-muted
                     focus:border-brand-blue focus:outline-none"
            />
            <button
              type="button"
              onclick={() => removeRow(i)}
              aria-label="Remove memory key {i + 1}"
              class="rounded border border-brand-border p-2 text-brand-muted
                     transition-colors hover:bg-brand-light-blue"
            >
              <X size={14} />
            </button>
          </div>
        {/each}
      </div>
    {:else}
      <p class="mt-3 text-xs text-brand-muted">
        No memory keys — the Memory tab stays hidden for players.
      </p>
    {/if}

    <button
      type="button"
      onclick={addRow}
      disabled={rows.length >= MAX_KEYS}
      data-testid="add-memory-key"
      class="mt-3 flex items-center gap-1.5 rounded-btn border border-brand-border px-3 py-1.5
             text-sm text-brand-text transition-colors hover:bg-brand-light-blue
             disabled:cursor-not-allowed disabled:opacity-50"
    >
      <Plus size={14} /> Add memory key
    </button>
  {/if}

  <input type="hidden" name="memory_schema_json" value={serialized} />
</div>
