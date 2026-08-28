<script lang="ts">
  import { ikAvatar } from '$lib/imagekit';
  import type { MemberInfo } from "$lib/types/arena";

  interface Props {
    participants: MemberInfo[];
  }

  let { participants }: Props = $props();
</script>

<section class="rounded-[8px] bg-white px-[32px] py-[28px]">
  <div class="mb-[14px] flex items-center justify-between">
    <h3 class="font-heading text-[14px] font-semibold" style="color: #363636;">Connected players</h3>
    <span
      class="rounded-full px-[8px] py-[2px] text-[11px] font-semibold"
      style="background: #f4f8fe; color: #8fb4ec;"
    >{participants.length}</span>
  </div>
  {#if participants.length === 0}
    <p class="text-[13px]" style="color: #8fb4ec;">No players connected yet.</p>
  {:else}
    <ul class="space-y-[8px]">
      {#each participants as player (player.user_id)}
        {@const href = player.username ? `/u/${player.username}` : null}
        <li>
          <svelte:element
            this={href ? "a" : "div"}
            {href}
            class="flex items-center gap-[10px] rounded-[6px] px-[8px] py-[6px] transition-colors {href ? 'hover:bg-[#f4f8fe] cursor-pointer' : ''}"
          >
            {#if player.avatar_url}
              <img
                src={ikAvatar(player.avatar_url, 32)}
                alt="{player.display_name} avatar"
                class="h-[32px] w-[32px] shrink-0 rounded-full object-cover"
              />
            {:else}
              <span
                class="inline-flex h-[32px] w-[32px] shrink-0 items-center justify-center rounded-full text-[13px] font-semibold text-white"
                style="background: #8fb4ec;"
              >
                {player.display_name.charAt(0).toUpperCase()}
              </span>
            {/if}
            <div class="min-w-0">
              <p class="truncate text-[14px] font-medium" style="color: #363636;">{player.display_name}</p>
              {#if player.agent_display_name}
                <p class="truncate text-[11px]" style="color: #8fb4ec;">{player.agent_display_name}</p>
              {/if}
            </div>
          </svelte:element>
        </li>
      {/each}
    </ul>
  {/if}
</section>
