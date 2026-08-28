<!--
  Hands the surrounding block to a coding agent in one click.

  Under a clock, retyping a requirement and a failing command is the most
  expensive thing on a session page, so this exists wherever a probe is shown.
  It reports what actually happened: a browser can refuse clipboard access on
  an insecure origin or a denied permission, and claiming success then would
  send the player off to paste nothing.
-->
<script lang="ts">
  import { Check, Copy } from 'lucide-svelte';

  interface Props {
    /** The markdown to place on the clipboard. */
    text: string;
    /** Smaller variant, for a row that is already dense. */
    compact?: boolean;
    testid?: string;
    class?: string;
  }

  let { text, compact = false, testid = 'copy-briefing', class: extra = '' }: Props = $props();

  /** null when idle; otherwise what the button reports about the last try. */
  let state = $state<'copied' | 'failed' | null>(null);
  let timer: ReturnType<typeof setTimeout> | null = null;

  async function copy() {
    if (timer) clearTimeout(timer);
    try {
      await navigator.clipboard.writeText(text);
      state = 'copied';
    } catch {
      state = 'failed';
    }
    timer = setTimeout(() => (state = null), 2500);
  }

  $effect(() => () => {
    if (timer) clearTimeout(timer);
  });

  const size = $derived(compact ? 12 : 14);
</script>

<button
  type="button"
  onclick={copy}
  data-testid={testid}
  title="Copy the task and this probe as markdown"
  class="flex items-center gap-1.5 rounded-btn border border-brand-border font-semibold
         text-brand-text transition-colors hover:border-brand-blue hover:text-brand-blue
         {compact ? 'px-2 py-0.5 text-[10px]' : 'px-2.5 py-1 text-xs'} {extra}"
>
  {#if state === 'copied'}
    <Check size={size} /> Copied
  {:else if state === 'failed'}
    <Copy size={size} /> Copy failed
  {:else}
    <Copy size={size} /> Copy for agent
  {/if}
</button>
