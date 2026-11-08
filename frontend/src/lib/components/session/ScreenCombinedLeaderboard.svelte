<script lang="ts">
  import type { SessionLinks } from "$lib/api";
  import { ikAvatar } from "$lib/imagekit";
  import { sessionLabel } from "$lib/session-links";

  let {
    links,
    emptyMessage = "No results recorded.",
  }: {
    /** The group whose standings to show: every player of every session, by points. */
    links: SessionLinks;
    emptyMessage?: string;
  } = $props();

  const sessions = $derived(links.sessions);
  const rows = $derived(links.leaderboard);

  // A workshop's finale: a few rooms' worth of players on one screen. A
  // dozen rows fit a board one under the other; past that the list runs in
  // two columns, past two dozen the rows shrink, and past what fits it
  // scrolls — the host can still walk the room through it.
  const twoColumns = $derived(rows.length > 12);
  const dense = $derived(rows.length > 24);
  const rankColor = (idx: number) =>
    idx === 0 ? "#f5a623" : idx === 1 ? "#9fb3cc" : idx === 2 ? "#bd8560" : "var(--s-dim)";

  const labelOf = (sessionId: string) => {
    const i = sessions.findIndex((s) => s.id === sessionId);
    return i < 0 ? "" : sessionLabel(sessions, i);
  };
  /** Where the points came from: the player's session — or, for the rare
   * player who sat in several, each session with its points. */
  function origin(row: SessionLinks["leaderboard"][number]): string {
    if (row.per_session.length === 1) return labelOf(row.per_session[0].session_id);
    return row.per_session.map((p) => `${labelOf(p.session_id)} ${p.points}`).join(" · ");
  }
</script>

<!-- The whole event's standings as a room reads them: rank, face, name,
     the session the player was in, the points — every room's players in
     one list, sorted by what they earned. -->
<ol
  class="combined flex h-full min-h-0 flex-col overflow-y-auto"
  class:two-columns={twoColumns}
  data-testid="screen-combined"
  data-columns={twoColumns ? "2" : "1"}
>
  {#each rows as row, idx (row.user_id ?? `${row.display_name}-${idx}`)}
    {@const size = dense ? 28 : 40}
    <li
      class="flex items-center gap-[12px] rounded-[10px] px-[12px]"
      class:py-[6px]={dense}
      class:py-[10px]={!dense}
      style={idx === 0 ? "background: var(--s-top);" : ""}
      data-testid="screen-combined-row"
    >
      <span
        class="w-[1.6em] shrink-0 text-center font-bold tabular-nums"
        style="color: {rankColor(idx)}; font-size: {dense ? 'clamp(13px,1.1vw,18px)' : 'clamp(16px,1.4vw,24px)'};"
      >{idx + 1}</span>
      {#if row.avatar_url}
        <img src={ikAvatar(row.avatar_url, size)} alt="" class="shrink-0 rounded-full object-cover" style="height: {size}px; width: {size}px;" />
      {:else}
        <span
          class="inline-flex shrink-0 items-center justify-center rounded-full font-semibold"
          style="height: {size}px; width: {size}px; background: var(--s-avatar); color: var(--s-avatar-text); font-size: {size * 0.42}px;"
        >{row.display_name.charAt(0).toUpperCase()}</span>
      {/if}
      <div class="min-w-0 flex-grow leading-tight">
        <p
          class="truncate font-semibold"
          style="font-size: {dense ? 'clamp(13px,1.1vw,18px)' : 'clamp(16px,1.4vw,24px)'};"
          title={row.display_name}
        >{row.display_name}</p>
        <p
          class="truncate"
          style="color: var(--s-head); font-size: {dense ? 'clamp(10px,0.8vw,13px)' : 'clamp(11px,0.9vw,15px)'};"
          title={origin(row)}
          data-testid="screen-combined-origin"
        >{origin(row)}</p>
      </div>
      <span
        class="shrink-0 text-right font-bold tabular-nums"
        style="color: {row.total_points >= 0 ? 'var(--s-pos)' : 'var(--s-neg)'}; font-size: {dense ? 'clamp(14px,1.2vw,20px)' : 'clamp(18px,1.6vw,28px)'};"
        data-testid="screen-combined-total"
      >{row.total_points}</span>
    </li>
  {:else}
    <li class="px-[12px] py-[10px] text-[clamp(14px,1.2vw,20px)]" style="color: var(--s-dim);">{emptyMessage}</li>
  {/each}
</ol>

<style>
  .combined.two-columns {
    display: block;
    column-count: 2;
    column-gap: 24px;
  }
  .combined.two-columns > li {
    break-inside: avoid;
  }
</style>
