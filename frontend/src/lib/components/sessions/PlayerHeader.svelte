<script lang="ts">
  import { ikAvatar } from '$lib/imagekit';
  import StatCard from "./StatCard.svelte";
  import { formatCountdown } from "$lib/format";
  import type { PlayerCompletionStatus } from "$lib/types/arena";

  let {
    playerName,
    avatarUrl = null,
    agentDisplayName,
    live,
    sessionFinished,
    sessionPaused,
    sessionCountdown,
    passedTasks,
    totalTasks,
    probesCount,
    nextProbeCountdown,
    score,
    rank,
    completionStatus = null,
    judgingPending = false,
    agentConnected = null,
  }: {
    playerName: string;
    avatarUrl?: string | null;
    agentDisplayName: string | null;
    live: boolean;
    sessionFinished: boolean;
    sessionPaused: boolean;
    sessionCountdown: number | null;
    /** Count of tasks the player has completed (not an ordinal). */
    passedTasks: number;
    totalTasks: number;
    probesCount: number;
    nextProbeCountdown: number | null;
    score: number;
    rank: number | null;
    /** Derived per-player completion state; null/absent renders no badge. */
    completionStatus?: PlayerCompletionStatus | null;
    /**
     * Judges still owe verdicts for this player. Suppresses the green
     * session "Complete" chip: the timer stopping is not completion while
     * the standings can still move.
     */
    judgingPending?: boolean;
    /**
     * Whether the player's ololo client is connected right now. null = the
     * server did not say (older backend) — keep the legacy session-only badge.
     */
    agentConnected?: boolean | null;
  } = $props();

  function getInitials(name: string): string {
    const parts = name.trim().split(/\s+/);
    if (parts.length >= 2)
      return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
    return name.slice(0, 2).toUpperCase();
  }
</script>

<!-- PLAYER HEADER -->
<div
  class="mb-8 flex flex-col gap-5 sm:flex-row sm:items-start sm:justify-between"
>
  <!-- Left: avatar + name + badge -->
  <div class="flex items-center gap-4">
    {#if avatarUrl}
      <img
        src={ikAvatar(avatarUrl, 64)}
        alt="{playerName} avatar"
        class="h-16 w-16 shrink-0 rounded-full object-cover"
      />
    {:else}
      <div
        class="flex h-16 w-16 shrink-0 items-center justify-center rounded-full
               bg-brand-blue/10 font-heading text-xl font-bold text-brand-blue"
      >
        {getInitials(playerName)}
      </div>
    {/if}
    <div>
      <div class="flex flex-wrap items-center gap-2.5">
        <h1
          class="font-heading text-[28px] font-bold leading-tight text-brand-text"
        >
          {playerName}
        </h1>
        {#if live && !sessionFinished && !sessionPaused && agentConnected !== false}
          <span
            class="inline-flex items-center gap-1.5 rounded-full bg-red-100
                   px-2.5 py-0.5 text-[11px] font-semibold text-red-600"
          >
            <span
              class="live-dot inline-block h-1.5 w-1.5 rounded-full bg-red-500"
            ></span>
            Live
          </span>
        {/if}
        {#if live && !sessionFinished && !sessionPaused && agentConnected === false}
          <!-- The session is running but this player's ololo client dropped:
               claiming "Live" here is exactly the lie this badge used to tell. -->
          <span
            class="inline-flex items-center gap-1.5 rounded-full bg-gray-200
                   px-2.5 py-0.5 text-[11px] font-semibold text-gray-600"
          >
            <span class="inline-block h-1.5 w-1.5 rounded-full bg-gray-400"></span>
            Agent offline
          </span>
        {/if}
        {#if live && sessionPaused}
          <span
            class="inline-flex items-center rounded-full bg-amber-100
                   px-2.5 py-0.5 text-[11px] font-semibold text-amber-700"
          >
            Paused
          </span>
        {/if}
        <!-- One coherent badge set: the session chip is dropped when the
             player chip already says "Completed" (redundant), and a player
             "In progress" chip is dropped once the session is over
             (contradictory — nothing is in progress anymore). -->
        {#if sessionFinished && completionStatus !== "completed" && completionStatus !== "awaiting_judges" && !judgingPending}
          <span
            class="inline-flex items-center rounded-full bg-green-100
                   px-2.5 py-0.5 text-[11px] font-semibold text-green-700"
          >
            Complete
          </span>
        {/if}
        {#if completionStatus === "in_progress" && !sessionFinished}
          <span
            class="inline-flex items-center rounded-full bg-brand-blue/10
                   px-2.5 py-0.5 text-[11px] font-semibold text-brand-blue"
          >
            In progress
          </span>
        {:else if completionStatus === "awaiting_judges"}
          <span
            class="inline-flex items-center rounded-full bg-amber-100
                   px-2.5 py-0.5 text-[11px] font-semibold text-amber-700"
          >
            Awaiting judges
          </span>
        {:else if completionStatus === "completed"}
          <span
            class="inline-flex items-center rounded-full bg-green-100
                   px-2.5 py-0.5 text-[11px] font-semibold text-green-700"
          >
            Completed
          </span>
        {/if}
      </div>
      {#if agentDisplayName}
        <p class="mt-0.5 text-[14px] font-semibold text-brand-blue">{agentDisplayName}</p>
      {/if}
    </div>
  </div>

  <!-- Right: session countdown + next probe + score + rank + tasks + probes stat cards -->
  <div class="flex shrink-0 items-center gap-3">
    {#if sessionPaused}
      <StatCard label="Session" value="Paused" valueClass="text-[18px] text-amber-600" />
    {:else if sessionCountdown !== null}
      <StatCard
        label="Time left"
        value={formatCountdown(sessionCountdown)}
        valueClass={sessionCountdown <= 30
          ? "text-[26px] text-red-600"
          : sessionCountdown <= 120
            ? "text-[26px] text-amber-600"
            : "text-[26px] text-green-600"}
      />
    {/if}
    <StatCard
      label="Tasks"
      value="{passedTasks}/{totalTasks}"
    />
    <StatCard label="Probes" value={String(probesCount)} />
    {#if !sessionPaused && nextProbeCountdown !== null}
      <StatCard
        label="Next probe"
        value="{nextProbeCountdown}s"
        valueClass={nextProbeCountdown <= 5
          ? "text-[26px] text-red-600"
          : nextProbeCountdown <= 15
            ? "text-[26px] text-amber-600"
            : "text-[26px] text-green-600"}
      />
    {/if}
    <StatCard
      label="Score"
      value={String(score)}
      valueClass={score >= 0 ? "text-green-600" : "text-red-600"}
    />
    <StatCard label="Rank" value="#{rank}" />
  </div>
</div>

<style>
  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.3;
    }
  }

  .live-dot {
    animation: pulse 1.5s ease-in-out infinite;
  }
</style>
