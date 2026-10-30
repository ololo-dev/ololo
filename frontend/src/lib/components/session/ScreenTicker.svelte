<script lang="ts">
  import type { ActivityEvent } from "$lib/types/arena";
  import { ikAvatar } from "$lib/imagekit";
  import { latestEvents, tickerLine } from "$lib/session-ticker";

  let {
    events,
    avatarByPlayerId = new Map<string, string | null>(),
    limit = 4,
  }: {
    events: ActivityEvent[];
    avatarByPlayerId?: Map<string, string | null>;
    /** How many of the latest events the strip holds. */
    limit?: number;
  } = $props();

  const rows = $derived(latestEvents(events, limit).map((ev) => ({ ev, line: tickerLine(ev) })));
</script>

<!-- The last few things that happened, newest first — the live strip under
     the board on the presentation screen. A new row fades in at the top;
     the oldest drops off the bottom. Colours come from the screen's palette. -->
<div class="flex min-h-[48px] flex-col gap-[6px]" data-testid="screen-ticker">
  {#if rows.length === 0}
    <p class="text-[clamp(14px,1.2vw,20px)]" style="color: var(--s-dim);">Waiting for the first move…</p>
  {:else}
    {#each rows as { ev, line } (ev.timestamp + ev.player_id + ev.task_id + ev.kind)}
      {@const avatar = avatarByPlayerId.get(ev.player_id)}
      <div class="ticker-row flex items-center gap-[12px] text-[clamp(14px,1.2vw,20px)] leading-tight">
        {#if avatar}
          <img src={ikAvatar(avatar, 28)} alt="" class="h-[28px] w-[28px] shrink-0 rounded-full object-cover" />
        {:else}
          <span
            class="inline-flex h-[28px] w-[28px] shrink-0 items-center justify-center rounded-full text-[13px] font-bold"
            style="background: var(--s-avatar); color: var(--s-avatar-text);"
          >{line.who.charAt(0).toUpperCase()}</span>
        {/if}
        <span class="shrink-0 font-semibold" style="color: var(--s-text);">{line.who}</span>
        <span class="min-w-0 truncate" style="color: var(--s-sub);">{line.what}</span>
        {#if line.delta != null}
          <span
            class="shrink-0 font-bold tabular-nums"
            style="color: {line.delta >= 0 ? 'var(--s-pos)' : 'var(--s-neg)'};"
          >{line.delta >= 0 ? "+" : ""}{line.delta}</span>
        {/if}
      </div>
    {/each}
  {/if}
</div>

<style>
  .ticker-row {
    animation: ticker-in 400ms ease-out;
  }
  @keyframes ticker-in {
    from { opacity: 0; transform: translateY(-6px); }
    to { opacity: 1; transform: none; }
  }
</style>
