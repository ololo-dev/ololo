<script lang="ts">
  import { browser } from '$app/environment';

  interface Props {
    code: string;
    class?: string;
  }

  let { code, class: extraClass = '' }: Props = $props();

  let copied = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;

  async function copy() {
    if (!browser) return;
    try {
      await navigator.clipboard.writeText(code);
      copied = true;
      clearTimeout(timer);
      timer = setTimeout(() => (copied = false), 1500);
    } catch {
      // clipboard unavailable
    }
  }
</script>

<div
  class="relative overflow-hidden rounded-[8px] {extraClass}"
  style="background-color: #36547f;"
>
  <!-- red left bar -->
  <div
    class="absolute bottom-0 left-0 top-0 w-[8px] rounded-l-[8px]"
    style="background-color: #fb341c;"
  ></div>

  <!-- copy button -->
  <button
    type="button"
    onclick={copy}
    aria-label={copied ? 'Copied!' : 'Copy to clipboard'}
    title={copied ? 'Copied!' : 'Copy to clipboard'}
    class="absolute right-[12px] top-[12px] flex h-[28px] w-[28px] items-center justify-center rounded-[4px] bg-white/10 text-white/60 transition-all hover:bg-white/20 hover:text-white"
  >
    {#if copied}
      <!-- checkmark -->
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width="14"
        height="14"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2.5"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <polyline points="20 6 9 17 4 12" />
      </svg>
    {:else}
      <!-- clipboard -->
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width="14"
        height="14"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <rect x="9" y="2" width="6" height="4" rx="1" />
        <path d="M8 4H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V6a2 2 0 0 0-2-2h-2" />
      </svg>
    {/if}
  </button>

  <pre
    class="overflow-x-auto py-[20px] pl-[40px] pr-[52px]"
  ><code class="font-mono text-[14px] leading-[1.5] text-white">{code}</code></pre>
</div>
