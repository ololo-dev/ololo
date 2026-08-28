<script lang="ts">
  // Replay control bar for a finished session: a playhead sweeps session time
  // while the score chart draws in and the activity feed reveals events up to
  // it. One-way props + callbacks (never bind: on a component).
  let {
    total,
    t,
    playing,
    speed,
    speeds = [1, 2, 4, 8],
    onSeek,
    onToggle,
    onSpeed,
  }: {
    /** Session length to replay, in elapsed seconds. */
    total: number;
    /** Current playhead, in elapsed seconds. */
    t: number;
    playing: boolean;
    speed: number;
    speeds?: number[];
    onSeek: (t: number) => void;
    onToggle: () => void;
    onSpeed: (s: number) => void;
  } = $props();

  function fmt(secs: number): string {
    let s = Math.max(0, Math.round(secs));
    const h = Math.floor(s / 3600);
    s -= h * 3600;
    const m = Math.floor(s / 60);
    const ss = String(s % 60).padStart(2, "0");
    // Hours only when the session is long enough to need them (real-time 1×
    // replays span the full session, which can run to hours).
    return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
  }

  const atEnd = $derived(t >= total);
</script>

<!-- Pinned like a media player so the controls stay reachable while the page
     (and, on the player view, the chat) scrolls through the replay. -->
<div class="fixed bottom-[16px] left-1/2 z-40 flex w-[calc(100%-24px)] max-w-[920px] -translate-x-1/2 flex-wrap items-center gap-[14px] rounded-[12px] border border-[#e3ecf9] bg-white px-[20px] py-[12px] shadow-[0_8px_30px_rgba(19,101,218,0.18)]">
  <button
    type="button"
    onclick={onToggle}
    aria-label={playing ? "Pause replay" : atEnd ? "Restart replay" : "Play replay"}
    data-testid="replay-toggle"
    class="flex h-[34px] w-[34px] shrink-0 items-center justify-center rounded-full bg-brand-blue text-white transition-opacity hover:opacity-85"
  >
    {#if playing}
      <svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="5" width="4" height="14" rx="1" /><rect x="14" y="5" width="4" height="14" rx="1" /></svg>
    {:else if atEnd}
      <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" /><path d="M3 3v5h5" /></svg>
    {:else}
      <svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="currentColor"><path d="M7 5v14l12-7z" /></svg>
    {/if}
  </button>

  <span class="text-[13px] font-semibold text-brand-muted">Replay</span>

  <input
    type="range"
    min="0"
    max={Math.max(1, total)}
    step="1"
    value={Math.min(t, total)}
    oninput={(e) => onSeek(Number((e.currentTarget as HTMLInputElement).value))}
    aria-label="Replay position"
    data-testid="replay-scrubber"
    class="replay-range h-[4px] min-w-[120px] flex-1 cursor-pointer appearance-none rounded-full"
    style="background: linear-gradient(to right, #0269fb {(Math.min(t, total) / Math.max(1, total)) * 100}%, #dce9fc {(Math.min(t, total) / Math.max(1, total)) * 100}%);"
  />

  <span class="shrink-0 text-[12px] tabular-nums text-brand-muted" data-testid="replay-time">
    {fmt(t)} / {fmt(total)}
  </span>

  <div class="flex shrink-0 items-center gap-[4px]">
    {#each speeds as s (s)}
      <button
        type="button"
        onclick={() => onSpeed(s)}
        data-testid="replay-speed-{s}"
        class="rounded-[6px] px-[8px] py-[3px] text-[12px] font-semibold transition-colors
          {s === speed ? 'bg-brand-blue text-white' : 'bg-brand-light-blue text-[#3061ac] hover:text-brand-text'}"
      >{s}×</button>
    {/each}
  </div>
</div>

<style>
  .replay-range::-webkit-slider-thumb {
    appearance: none;
    width: 14px;
    height: 14px;
    border-radius: 9999px;
    background: #0269fb;
    border: 2px solid #fff;
    box-shadow: 0 1px 3px rgba(2, 105, 251, 0.4);
    cursor: pointer;
  }
  .replay-range::-moz-range-thumb {
    width: 14px;
    height: 14px;
    border: 2px solid #fff;
    border-radius: 9999px;
    background: #0269fb;
    cursor: pointer;
  }
</style>
