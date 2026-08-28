<script lang="ts">
  type Props = {
    src: string;
    alt: string;
    onclose: () => void;
    /** Step callbacks: when present, ‹ › buttons and arrow keys navigate. */
    onprev?: (() => void) | null;
    onnext?: (() => void) | null;
    /** Position label, e.g. "3 / 9" — shown when navigating a gallery. */
    counter?: string | null;
  };
  let { src, alt, onclose, onprev = null, onnext = null, counter = null }: Props = $props();

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') onclose();
    else if (e.key === 'ArrowLeft' && onprev) onprev();
    else if (e.key === 'ArrowRight' && onnext) onnext();
  }
</script>

<svelte:window onkeydown={onKeydown} />

<!-- Full-viewport overlay: any click outside the image closes it. -->
<div
  class="fixed inset-0 z-50 flex items-center justify-center bg-black/80 p-6"
  data-testid="image-lightbox"
  role="dialog"
  aria-modal="true"
  aria-label={alt}
>
  <button
    type="button"
    class="absolute inset-0 h-full w-full cursor-zoom-out"
    aria-label="Close image"
    onclick={onclose}
  ></button>
  <figure class="pointer-events-none relative z-10 max-h-full max-w-full">
    <img {src} {alt} class="max-h-[85vh] max-w-full rounded-lg shadow-2xl" />
    <figcaption class="mt-2 text-center font-mono text-[11px] text-white/70">
      {alt}{#if counter}<span class="ml-2 text-white/50">{counter}</span>{/if}
    </figcaption>
  </figure>
  {#if onprev}
    <button
      type="button"
      class="absolute left-4 top-1/2 z-20 -translate-y-1/2 rounded-full bg-white/10 px-3 py-2 text-[18px] font-semibold text-white transition-colors hover:bg-white/25"
      aria-label="Previous image"
      data-testid="lightbox-prev"
      onclick={onprev}
    >‹</button>
  {/if}
  {#if onnext}
    <button
      type="button"
      class="absolute right-4 top-1/2 z-20 -translate-y-1/2 rounded-full bg-white/10 px-3 py-2 text-[18px] font-semibold text-white transition-colors hover:bg-white/25"
      aria-label="Next image"
      data-testid="lightbox-next"
      onclick={onnext}
    >›</button>
  {/if}
  <button
    type="button"
    class="absolute right-4 top-4 z-20 rounded-full bg-white/10 px-3 py-1 text-[13px] font-semibold text-white transition-colors hover:bg-white/25"
    onclick={onclose}
  >Close ✕</button>
</div>
