<script module lang="ts">
  /** Personal result block shown on the player page's popup. */
  export interface CongratsSummary {
    place: number;
    of: number;
    points: number;
    testsPassed: number;
  }
</script>

<script lang="ts">
  import Modal from '$lib/components/ui/Modal.svelte';
  import type { LeaderboardEntry } from '$lib/types/arena';

  let {
    open = false,
    onClose,
    entries = [],
    highlightPlayerId = null,
    summary = null,
  }: {
    open?: boolean;
    onClose?: () => void;
    entries?: LeaderboardEntry[];
    highlightPlayerId?: string | null;
    summary?: CongratsSummary | null;
  } = $props();

  const standings = $derived([...entries].sort((a, b) => b.total_points - a.total_points));
  const medals = ['🥇', '🥈', '🥉'];
</script>

<Modal {open} {onClose}>
  <div class="text-center" data-testid="session-congrats">
    <div class="text-[44px] leading-none" aria-hidden="true">🎉</div>
    <h2 class="mt-3 font-heading text-[24px] font-bold text-brand-text">Congratulations!</h2>
    <p class="mt-1 text-[14px] text-brand-muted">
      The session is complete and every judge has spoken — the standings are final.
    </p>

    {#if summary}
      <div
        class="mx-auto mt-5 w-full max-w-[360px] rounded-xl border border-brand-border/60 bg-brand-light-blue/15 px-5 py-4 text-left"
        data-testid="congrats-summary"
      >
        <p class="text-[11px] font-semibold uppercase tracking-wide text-brand-muted">Your result</p>
        <div class="mt-2 flex items-baseline gap-2">
          <span class="font-heading text-[28px] font-bold text-brand-text">#{summary.place}</span>
          <span class="text-[13px] text-brand-muted">of {summary.of}</span>
          <span class="ml-auto text-[18px] font-bold tabular-nums text-brand-text">{summary.points} pts</span>
        </div>
        <div class="mt-1.5 flex flex-wrap items-center gap-x-4 gap-y-1 text-[12px] text-brand-muted">
          <span>{summary.testsPassed} {summary.testsPassed === 1 ? 'check' : 'checks'} passed</span>
        </div>
      </div>
    {/if}

    {#if standings.length > 0}
      <ol class="mx-auto mt-5 flex max-h-[300px] w-full max-w-[360px] flex-col gap-1.5 overflow-y-auto pr-1 text-left">
        {#each standings as entry, i (entry.player_id)}
          <li
            class="flex items-center gap-3 rounded-lg px-4 py-2 {entry.player_id === highlightPlayerId
              ? 'bg-brand-blue/10 ring-1 ring-brand-blue/40'
              : i === 0
                ? 'bg-amber-50'
                : 'bg-brand-light-blue/20'} {i === 0 ? 'font-semibold' : ''}"
          >
            <span class="w-7 shrink-0 text-[14px]" aria-hidden="true">{medals[i] ?? `${i + 1}.`}</span>
            <span class="min-w-0 flex-1 truncate text-[14px] text-brand-text">{entry.display_name}</span>
            <span class="shrink-0 text-[14px] font-bold tabular-nums text-brand-text">{entry.total_points} pts</span>
          </li>
        {/each}
      </ol>
    {/if}

    <button
      type="button"
      class="mt-6 rounded-btn bg-brand-blue px-6 py-2 text-[14px] font-semibold text-white transition-opacity hover:opacity-85"
      onclick={onClose}
    >
      See the full results
    </button>
  </div>
</Modal>
