<!--
  One ranked player as a card row: medal badge, avatar, name + meta, score.
  Used by the project "Top Players" board.
-->
<script lang="ts">
  import { ikAvatar } from '$lib/imagekit';

  let {
    rank,
    displayName,
    username = null,
    avatarUrl = null,
    points,
    meta = '',
    unit = 'pts',
    children,
  }: {
    rank: number;
    displayName: string;
    username?: string | null;
    avatarUrl?: string | null;
    points: number;
    /** Secondary line under the name, e.g. "5 sessions · best finish #1". */
    meta?: string;
    /** Unit shown under the score. */
    unit?: string;
    /** Extra marks on the meta line — the campaign board hangs a progress
     *  chip here rather than folding it into the meta sentence. */
    children?: import('svelte').Snippet;
  } = $props();

  function initials(name: string): string {
    const parts = name.trim().split(/\s+/).slice(0, 2);
    return parts.map((p) => p[0]?.toUpperCase() ?? '').join('') || '?';
  }

  // Medal tint for the podium; everyone else gets the flat accent chip.
  function rankStyle(r: number): string {
    if (r === 1) return 'background:#f6c94a;color:#5a4300;';
    if (r === 2) return 'background:#cdd4dd;color:#3a4048;';
    if (r === 3) return 'background:#e2a774;color:#4a2c11;';
    return 'background:#e2ecfc;color:#8fb4ec;';
  }
</script>

<li class="flex items-center gap-[16px] rounded-[8px] bg-white px-[24px] py-[16px]">
  <span
    class="flex h-[32px] w-[32px] shrink-0 items-center justify-center rounded-full text-[14px] font-bold tabular-nums"
    style={rankStyle(rank)}
  >
    {rank}
  </span>

  {#if avatarUrl}
    <img
      src={ikAvatar(avatarUrl, 40)}
      alt=""
      class="h-[40px] w-[40px] shrink-0 rounded-full object-cover"
    />
  {:else}
    <span
      class="flex h-[40px] w-[40px] shrink-0 items-center justify-center rounded-full bg-brand-light-blue text-[14px] font-bold text-brand-blue"
    >
      {initials(displayName)}
    </span>
  {/if}

  <div class="min-w-0 flex-1">
    {#if username}
      <a
        href="/u/{username}"
        class="block truncate text-[16px] font-semibold text-brand-text hover:text-brand-blue"
      >
        {displayName}
      </a>
    {:else}
      <p class="truncate text-[16px] font-semibold text-brand-text">{displayName}</p>
    {/if}
    {#if meta || children}
      <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
        {#if meta}
          <p class="text-[13px] text-brand-muted">{meta}</p>
        {/if}
        {@render children?.()}
      </div>
    {/if}
  </div>

  <div class="shrink-0 text-right">
    <!-- Judges can push a score below zero; red keeps that from reading as a
         win at a glance. -->
    <p class="text-[20px] font-bold tabular-nums" style="color: {points < 0 ? '#e5484d' : '#363636'};">
      {points}
    </p>
    <p class="text-[12px] font-semibold uppercase tracking-wide text-brand-muted">{unit}</p>
  </div>
</li>
