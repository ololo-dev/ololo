<script lang="ts">
  // A judge verdict in the activity feed: rendered as markdown, clamped to
  // three lines, with a toggle that appears only when the clamp actually
  // hides something.
  //
  // Markdown because that is what the judges write — verdicts name files and
  // symbols in backticks (`src/bin/server.rs`, `handle_session`) and lean on
  // emphasis and lists, all of which used to reach the reader as raw syntax.
  //
  // Measured because whether three lines hide anything depends on the
  // rendered width, not on the character count: in a ~990px feed a
  // 440-character verdict fits, so a length-based heuristic offered "Show
  // more" on half the rows and clicking it changed nothing.

  import MarkdownContent from "$lib/components/MarkdownContent.svelte";

  let { text }: { text: string } = $props();

  let box = $state<HTMLDivElement | null>(null);
  let expanded = $state(false);
  let clipped = $state(false);

  $effect(() => {
    // The clamp sits on the rendered-markdown element, not on this wrapper —
    // the wrapper's own height just follows its clamped child, so measuring it
    // would never see an overflow.
    const node = box?.firstElementChild as HTMLElement | undefined;
    // Nothing to measure while unclamped: `clipped` keeps the value taken on
    // the way in, which is what "Show less" needs to stay on screen.
    if (!node || expanded) return;

    let live = true;
    const measure = () => {
      if (!live) return;
      clipped = node.scrollHeight > node.clientHeight + 1;
    };
    measure();

    const ro = new ResizeObserver(measure);
    ro.observe(node);
    // Webfonts rewrap the text after they swap in — measure once more when
    // they land.
    document.fonts?.ready.then(measure);

    return () => {
      live = false;
      ro.disconnect();
    };
  });
</script>

<div bind:this={box} class="max-w-prose" class:clamped={!expanded}>
  <MarkdownContent value={text} class="verdict-md !text-[13px] !leading-[1.6]" />
</div>
{#if clipped}
  <button
    type="button"
    class="mt-[4px] text-[12px] font-semibold text-brand-blue hover:underline"
    onclick={() => (expanded = !expanded)}
  >
    {expanded ? "Show less" : "Show more"}
  </button>
{/if}

<style>
  /* Three lines of the flowed markdown, ellipsised. The clamp goes on the
     rendered-markdown child, which is the element that holds the text flow. */
  .clamped :global(> div) {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    overflow: hidden;
  }
</style>
