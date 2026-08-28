<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    open?: boolean;
    onClose?: () => void;
    children?: Snippet;
    maxWidth?: 'sm' | 'md';
  }

  let {
    open = false,
    onClose,
    children,
    maxWidth = 'sm',
  }: Props = $props();

  function handleBackdrop(e: MouseEvent) {
    if (e.target === e.currentTarget) onClose?.();
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') onClose?.();
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="fixed inset-0 z-[100] flex overflow-auto bg-[rgba(54,84,127,0.4)] sm:p-10"
    onclick={handleBackdrop}
  >
    <!-- Full-screen sheet on phones; centered card from sm up. m-auto (not
         items-center) so a panel taller than the viewport scrolls from its top
         instead of clipping. -->
    <div
      class={[
        'relative m-auto min-h-full w-full bg-white px-5 py-14 text-left sm:min-h-0 sm:py-12',
        maxWidth === 'sm' ? 'sm:max-w-[570px] sm:px-[100px]' : 'sm:max-w-[970px] sm:px-16',
      ].join(' ')}
    >
      <!-- Close button -->
      <button
        type="button"
        class="absolute right-3 top-3 z-10 flex h-10 w-10 items-center justify-center bg-transparent transition-opacity hover:opacity-60 sm:right-5 sm:top-5"
        aria-label="Close"
        onclick={onClose}
      >
        <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24">
          <g fill="none" fill-rule="evenodd">
            <path
              fill="#DCE9FC"
              fill-rule="nonzero"
              d="M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"
            />
          </g>
        </svg>
      </button>

      {@render children?.()}
    </div>
  </div>
{/if}
