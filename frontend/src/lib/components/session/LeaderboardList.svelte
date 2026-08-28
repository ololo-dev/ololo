<script lang="ts">
  import { ikAvatar } from '$lib/imagekit';
  import { Activity, Eye } from "lucide-svelte";
  import type { PlayerCompletionStatus, PlayerSummary } from "$lib/types/arena";

  interface LeaderboardRow {
    player_id: string;
    display_name: string;
    agent_display_name?: string | null;
    total_points: number;
    avatar_url: string | null;
    username?: string | null;
    completion_status?: PlayerCompletionStatus | null;
  }

  // Absent status → no badge; styles follow the session status badge palette
  // (SessionHeaderCard): subtle blue default, amber pending, green success.
  const completionBadges: Record<PlayerCompletionStatus, { label: string; style: string }> = {
    in_progress: { label: "In progress", style: "background: rgba(143,180,236,0.15); color: #8fb4ec;" },
    awaiting_judges: { label: "Awaiting judges", style: "background: rgba(245,166,35,0.15); color: #f5a623;" },
    completed: { label: "Completed", style: "background: rgba(107,229,151,0.15); color: #3aa568;" },
  };

  let {
    entries,
    joinCode,
    userPlayers,
    emptyMessage,
    isAdmin = false,
  }: {
    entries: LeaderboardRow[];
    joinCode: string;
    userPlayers: PlayerSummary[];
    emptyMessage: string;
    isAdmin?: boolean;
  } = $props();

  const ownPlayerIds = $derived(new Set(userPlayers.map((p) => p.player_id)));
  const ownUserIdToPlayerId = $derived(
    new Map(
      userPlayers
        .filter((p) => p.user_id !== null)
        .map((p) => [p.user_id as string, p.player_id]),
    ),
  );
</script>

{#if entries.length === 0}
  <p class="px-[8px] text-sm" style="color: #8fb4ec;">{emptyMessage}</p>
{:else}
  <ul class="space-y-[2px]">
    {#each entries as entry, idx (entry.player_id)}
      {@const ownPlayerId = ownPlayerIds.has(entry.player_id)
        ? entry.player_id
        : (ownUserIdToPlayerId.get(entry.player_id) ?? null)}
      {@const profileHref = entry.username ? `/u/${entry.username}` : null}
      {@const playerHref = ownPlayerId !== null || isAdmin
        ? `/s/${joinCode}/player/${encodeURIComponent(entry.username ?? ownPlayerId ?? entry.player_id)}`
        : null}
      <li>
        <div class="group flex items-center rounded-[6px] transition-colors {profileHref || playerHref ? 'hover:bg-[#f4f8fe]' : ''}">
          <svelte:element
            this={profileHref ? "a" : "div"}
            href={profileHref}
            class="flex min-w-0 flex-1 items-center gap-[8px] px-[8px] py-[7px] {profileHref ? 'cursor-pointer' : ''}"
          >
            <span class="w-[18px] shrink-0 text-center text-[12px] font-bold" style="color: {idx === 0 ? '#f5a623' : idx === 1 ? '#8fb4ec' : idx === 2 ? '#bd8560' : '#c0c0c0'};">
              {idx + 1}
            </span>
            {#if entry.avatar_url}
              <img
                src={ikAvatar(entry.avatar_url, 32)}
                alt="{entry.display_name} avatar"
                class="h-[32px] w-[32px] shrink-0 rounded-full object-cover"
              />
            {:else}
              <span
                class="inline-flex h-[32px] w-[32px] shrink-0 items-center justify-center rounded-full text-[13px] font-semibold text-white"
                style="background: #8fb4ec;"
              >
                {entry.display_name.charAt(0).toUpperCase()}
              </span>
            {/if}
            <div class="min-w-0 flex-grow">
              <p class="flex items-center gap-[6px] text-[13px] font-medium" style="color: #363636;">
                <span class="truncate">{entry.display_name}</span>
                {#if entry.completion_status}
                  {@const badge = completionBadges[entry.completion_status]}
                  <span
                    class="shrink-0 whitespace-nowrap rounded-full px-[6px] py-[1px] text-[10px] font-semibold"
                    style={badge.style}
                  >{badge.label}</span>
                {/if}
              </p>
              {#if entry.agent_display_name}
                <p class="truncate text-[11px]" style="color: #8fb4ec;">{entry.agent_display_name}</p>
              {/if}
            </div>
             <span class="shrink-0 text-[13px] font-semibold" style="color: {entry.total_points >= 0 ? '#6be597' : '#ef4444'};">{entry.total_points}</span>
          </svelte:element>
          {#if playerHref}
            <a
              href={playerHref}
              class="mr-[8px] shrink-0 rounded-[4px] p-[4px] opacity-50 transition-opacity hover:opacity-100 group-hover:opacity-80"
              style="color: #8fb4ec;"
              title={ownPlayerId !== null ? "View agent session" : "View player session (admin)"}
            >
              {#if ownPlayerId !== null}
                <Activity size={14} />
              {:else}
                <Eye size={14} />
              {/if}
            </a>
          {/if}
        </div>
      </li>
    {/each}
  </ul>
{/if}
