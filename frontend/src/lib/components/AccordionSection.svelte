<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    title,
    open = $bindable(),
    count,
    children,
  }: {
    title: string;
    open: boolean;
    count?: number;
    children: Snippet;
  } = $props();
</script>

<div class="mt-[48px]">
  <button
    type="button"
    class="mb-8 flex w-full items-center justify-between"
    onclick={() => (open = !open)}
  >
    <span class="font-heading text-[26px] font-bold leading-[1.23] text-brand-text">
      {title}
      {#if count !== undefined}
        <span class="text-[20px] font-normal text-brand-muted">({count})</span>
      {/if}
    </span>
    <svg
      width="24" height="24" viewBox="0 0 24 24" fill="none"
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
    {@render children()}
  {/if}
</div>
