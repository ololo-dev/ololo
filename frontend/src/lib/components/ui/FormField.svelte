<script lang="ts">
  import type { HTMLInputAttributes, HTMLTextareaAttributes } from 'svelte/elements';

  interface Props {
    label?: string;
    type?: HTMLInputAttributes['type'] | 'textarea';
    id?: string;
    name?: string;
    placeholder?: string;
    value?: string;
    error?: string;
    hint?: string;
    required?: boolean;
    disabled?: boolean;
    class?: string;
    onchange?: (value: string) => void;
  }

  let {
    label,
    type = 'text',
    id,
    name,
    placeholder,
    value = '',
    error,
    hint,
    required = false,
    disabled = false,
    class: className = '',
    onchange,
  }: Props = $props();

  // Stable fallback ID generated once; $derived picks up id/name changes reactively.
  const stableId = crypto.randomUUID();
  const inputId = $derived(id ?? name ?? stableId);

  const inputClasses = $derived(
    [
      'block w-full rounded-btn border-2 bg-white px-4 py-[5px] h-12',
      'font-body text-base text-brand-text placeholder-brand-muted',
      'transition-colors focus:outline-none focus:border-brand-blue',
      error ? 'border-brand-red' : 'border-brand-border',
    ].join(' ')
  );
</script>

<div class={['space-y-1', className].filter(Boolean).join(' ')}>
  {#if label}
    <label
      for={inputId}
      class="block cursor-pointer text-xs font-semibold leading-snug text-brand-text select-none"
    >
      {label}{#if required}<span class="text-brand-red ml-0.5">*</span>{/if}
    </label>
  {/if}

  {#if type === 'textarea'}
    <textarea
      id={inputId}
      {name}
      {placeholder}
      {required}
      {disabled}
      class={[inputClasses, 'h-36 resize-none py-3'].join(' ')}
      oninput={(e) => onchange?.(e.currentTarget.value)}
    >{value}</textarea>
  {:else}
    <input
      id={inputId}
      {name}
      {type}
      {placeholder}
      {required}
      {disabled}
      {value}
      class={inputClasses}
      oninput={(e) => onchange?.(e.currentTarget.value)}
    />
  {/if}

  {#if error}
    <p class="mt-0.5 block text-xs leading-none text-brand-red">{error}</p>
  {:else if hint}
    <p class="mt-0.5 block text-xs leading-none text-brand-muted">{hint}</p>
  {/if}
</div>
