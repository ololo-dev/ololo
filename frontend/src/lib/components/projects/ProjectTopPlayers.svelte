<script lang="ts">
  import TopPlayerRow from '$lib/components/projects/TopPlayerRow.svelte';
  import type { TopPlayer } from '$lib/api';

  interface Props {
    /** Every season the project has been played in. */
    players: TopPlayer[];
    /** The same board restricted to the current season. */
    seasonPlayers: TopPlayer[];
    /** Start of the current season (RFC 3339); empty when unknown. */
    seasonStart: string;
    /** Parts to clear, on a campaign board. Absent on ordinary projects,
     *  where a player's progress through the project is the score itself. */
    partsTotal?: number | null;
  }

  let { players, seasonPlayers, seasonStart, partsTotal = null }: Props = $props();

  type Board = 'all' | 'season';
  // All time leads: a season resets on the 1st, so the seasonal board is
  // empty for the first days of every month while the project is not.
  let board = $state<Board>('all');

  const seasonLabel = $derived(
    new Date(seasonStart || Date.now()).toLocaleDateString('en-US', {
      month: 'long',
      timeZone: 'UTC',
    }),
  );

  const shown = $derived(board === 'all' ? players : seasonPlayers);

  function meta(player: TopPlayer): string {
    const sessions = `${player.sessions_played} ${player.sessions_played === 1 ? 'session' : 'sessions'}`;
    return player.best_placement !== null
      ? `${sessions} · best finish #${player.best_placement}`
      : sessions;
  }

  // Points cannot answer "who actually got through this": a big run on part
  // one outscores a modest run of all five. On a campaign the board says how
  // far each player got, and marks the ones who finished.
  function progress(player: TopPlayer): { label: string; done: boolean } | null {
    if (!partsTotal || player.parts_completed == null) return null;
    const done = player.parts_completed >= partsTotal;
    return {
      label: done
        ? `Campaign complete · ${partsTotal} ${partsTotal === 1 ? 'part' : 'parts'}`
        : `${player.parts_completed} of ${partsTotal} parts done`,
      done,
    };
  }

  const tabClass = (active: boolean) =>
    `rounded-btn px-3 py-1 text-sm transition-colors ${
      active
        ? 'bg-brand-blue font-semibold text-white'
        : 'text-brand-text hover:bg-brand-light-blue'
    }`;
</script>

<div class="mt-[24px] flex items-center gap-[8px]" role="tablist" aria-label="Top players range">
  <button
    type="button"
    role="tab"
    aria-selected={board === 'all'}
    class={tabClass(board === 'all')}
    data-testid="top-players-all"
    onclick={() => (board = 'all')}
  >
    All time
    <span class={board === 'all' ? 'text-white/70' : 'text-brand-muted'}>({players.length})</span>
  </button>
  <button
    type="button"
    role="tab"
    aria-selected={board === 'season'}
    class={tabClass(board === 'season')}
    data-testid="top-players-season"
    onclick={() => (board = 'season')}
  >
    {seasonLabel}
    <span class={board === 'season' ? 'text-white/70' : 'text-brand-muted'}
      >({seasonPlayers.length})</span
    >
  </button>
</div>

{#if shown.length === 0}
  <p class="mt-[16px] text-brand-muted" data-testid="top-players-empty">
    {#if board === 'season' && players.length > 0}
      No finishes yet this {seasonLabel} — the all-time board still stands.
    {:else}
      No players yet — points appear here once sessions of this project finish.
    {/if}
  </p>
{:else}
  <ul class="mt-[16px] flex flex-col gap-[8px]">
    {#each shown as player (player.user_id)}
      {@const done = progress(player)}
      <TopPlayerRow
        rank={player.rank}
        displayName={player.display_name}
        username={player.username}
        avatarUrl={player.avatar_url}
        points={player.game_points}
        meta={meta(player)}
      >
        {#if done}
          <span
            class="inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[12px] font-semibold {done.done
              ? 'bg-emerald-50 text-emerald-700'
              : 'bg-brand-light-blue text-brand-blue'}"
            data-testid="campaign-progress-{player.user_id}"
          >
            {#if done.done}<span aria-hidden="true">🏁</span>{/if}
            {done.label}
          </span>
        {/if}
      </TopPlayerRow>
    {/each}
  </ul>
{/if}
