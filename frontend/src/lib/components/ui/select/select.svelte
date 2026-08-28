<script lang="ts">
  import { cn } from '$lib/utils';

  interface Option {
    value: string;
    label: string;
  }

  let {
    value = '',
    options = [] as Option[],
    placeholder = 'Select…',
    disabled = false,
    class: className = '',
    onchange,
  }: {
    value?: string;
    options?: Option[];
    placeholder?: string;
    disabled?: boolean;
    class?: string;
    onchange?: (value: string) => void;
  } = $props();

  function handleChange(e: Event) {
    const target = e.currentTarget as HTMLSelectElement;
    onchange?.(target.value);
  }
</script>

<select
  {disabled}
  value={value}
  onchange={handleChange}
  class={cn(
    'flex h-10 w-full rounded-md border border-slate-300 bg-transparent px-3 py-2 text-sm',
    'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate-900',
    'disabled:cursor-not-allowed disabled:opacity-50',
    className,
  )}
>
  {#if placeholder && !value}
    <option value="" disabled selected>{placeholder}</option>
  {/if}
  {#each options as opt (opt.value)}
    <option value={opt.value} selected={opt.value === value}>{opt.label}</option>
  {/each}
</select>
