<script lang="ts">
  import { ikAvatar } from "$lib/imagekit";
  import type { PlayerCompletionStatus } from "$lib/types/arena";
  import { LEVEL_COLORS, formatScore, type HealthIndicator } from "$lib/session-health";

  interface ScreenRow {
    player_id: string;
    display_name: string;
    avatar_url?: string | null;
    agent_display_name?: string | null;
    completion_status?: PlayerCompletionStatus | null;
    total_points: number;
    /** The latest code-health score, when the session tracks health. */
    health?: HealthIndicator | null;
  }

  const trendGlyph = (h: HealthIndicator) =>
    h.trend === "up" ? "▲" : h.trend === "down" ? "▼" : h.trend === "flat" ? "–" : "";

  let {
    entries,
    emptyMessage,
  }: {
    entries: ScreenRow[];
    emptyMessage: string;
  } = $props();

  const statusLabel: Record<PlayerCompletionStatus, string> = {
    in_progress: "in progress",
    awaiting_judges: "awaiting judges",
    completed: "completed",
  };

  // Past a dozen rows the board must shrink to stay on one screen: a
  // workshop seats sixteen, and a scrolling leaderboard is unreadable
  // through a video call.
  const dense = $derived(entries.length > 12);
  const rankColor = (idx: number) =>
    idx === 0 ? "#f5a623" : idx === 1 ? "#9fb3cc" : idx === 2 ? "#bd8560" : "var(--s-dim)";
</script>

<!-- The leaderboard as a room reads it: rank, face, name, points — no links,
     no pills, nothing to hover. Colours come from the screen's palette. -->
{#if entries.length === 0}
  <p class="text-[clamp(14px,1.2vw,20px)]" style="color: var(--s-dim);">{emptyMessage}</p>
{:else}
  <ol class="flex flex-col" style="gap: {dense ? 2 : 6}px;" data-testid="screen-leaderboard">
    {#each entries as entry, idx (entry.player_id)}
      {@const size = dense ? 28 : 40}
      <li
        class="flex items-center gap-[12px] rounded-[8px] px-[12px]"
        style="padding-top: {dense ? 3 : 8}px; padding-bottom: {dense ? 3 : 8}px; background: {idx === 0 ? 'var(--s-top)' : 'transparent'};"
      >
        <span
          class="w-[1.6em] shrink-0 text-center font-bold tabular-nums"
          style="color: {rankColor(idx)}; font-size: {dense ? 'clamp(13px,1.1vw,18px)' : 'clamp(16px,1.4vw,24px)'};"
        >{idx + 1}</span>
        {#if entry.avatar_url}
          <img
            src={ikAvatar(entry.avatar_url, size)}
            alt=""
            class="shrink-0 rounded-full object-cover"
            style="height: {size}px; width: {size}px;"
          />
        {:else}
          <span
            class="inline-flex shrink-0 items-center justify-center rounded-full font-semibold"
            style="height: {size}px; width: {size}px; background: var(--s-avatar); color: var(--s-avatar-text); font-size: {size * 0.42}px;"
          >{entry.display_name.charAt(0).toUpperCase()}</span>
        {/if}
        <div class="min-w-0 flex-grow leading-tight">
          <p
            class="truncate font-semibold"
            style="color: var(--s-text); font-size: {dense ? 'clamp(13px,1.1vw,18px)' : 'clamp(16px,1.4vw,24px)'};"
            title={entry.display_name}
          >{entry.display_name}</p>
          {#if !dense && (entry.agent_display_name || entry.completion_status)}
            <p class="truncate text-[clamp(11px,0.9vw,15px)]" style="color: var(--s-head);">
              {[entry.agent_display_name, entry.completion_status ? statusLabel[entry.completion_status] : null]
                .filter(Boolean)
                .join(" · ")}
            </p>
          {/if}
        </div>
        <div class="flex shrink-0 flex-col items-end leading-tight">
          <span
            class="font-bold tabular-nums"
            style="color: {entry.total_points >= 0 ? 'var(--s-pos)' : 'var(--s-neg)'}; font-size: {dense ? 'clamp(14px,1.2vw,20px)' : 'clamp(18px,1.6vw,28px)'};"
          >{entry.total_points}</span>
          {#if entry.health}
            {@const colors = LEVEL_COLORS[entry.health.level]}
            <!-- The score above, the code's health below: the grade in a
                 box (filled once server-verified), the number, the trend
                 since the previous check — HealthBadge's chip, sized for
                 a room. -->
            <span
              class="mt-[2px] inline-flex items-center gap-[4px] whitespace-nowrap rounded-full px-[7px] py-[1px] font-semibold tabular-nums"
              style="background: {colors.bg}; color: {colors.fg}; font-size: {dense ? 'clamp(10px,0.8vw,13px)' : 'clamp(11px,0.95vw,16px)'};"
              data-testid="screen-health"
              data-level={entry.health.level}
            >
              <span
                class="inline-flex items-center justify-center rounded-[3px] border px-[3px] font-bold leading-none"
                style="min-width: 1.3em; height: 1.3em; background: {entry.health.verified ? colors.fg : 'transparent'}; border-color: {colors.fg}; color: {entry.health.verified ? '#ffffff' : colors.fg};"
              >{entry.health.grade ?? "–"}</span>
              <span>{formatScore(entry.health.score)}</span>
              {#if trendGlyph(entry.health)}
                <span aria-label={entry.health.trend ?? undefined}>{trendGlyph(entry.health)}</span>
              {/if}
            </span>
          {/if}
        </div>
      </li>
    {/each}
  </ol>
{/if}
