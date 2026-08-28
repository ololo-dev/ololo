<script lang="ts">
  import type { Session } from "$lib/api";
  import type { WsProjectClient } from "$lib/ws-project.svelte";
  import { formatDateUTC, formatHms as fmtSecs, formatTimeUTC } from "$lib/format";

  interface SessionGroup {
    label: string;
    status: string;
    sessions: Session[];
  }

  let {
    sessions,
    sessionGroups,
    isAdmin,
    showCancelled = $bindable(),
    wsClient,
  }: {
    sessions: Session[];
    sessionGroups: SessionGroup[];
    isAdmin: boolean;
    showCancelled: boolean;
    wsClient: WsProjectClient | null;
  } = $props();

</script>

<!-- Sessions section (heading provided by the parent tab bar) -->
{#if isAdmin}
  <!-- Admin-only: toggle to show cancelled/failed -->
  <div class="mt-[20px] flex justify-end">
    <label
      class="flex cursor-pointer items-center gap-[8px] text-[13px] text-brand-muted select-none"
    >
      <span>Show cancelled</span>
      <button
        type="button"
        role="switch"
        aria-checked={showCancelled}
        aria-label="Show cancelled sessions"
        onclick={() => (showCancelled = !showCancelled)}
        class="relative inline-flex h-[20px] w-[36px] shrink-0 items-center rounded-full transition-colors"
        style="background: {showCancelled ? '#4a90e2' : '#d1d5db'};"
      >
        <span
          class="absolute inline-block h-[14px] w-[14px] rounded-full bg-white shadow transition-transform"
          style="transform: translateX({showCancelled ? '18px' : '3px'});"
        ></span>
      </button>
    </label>
  </div>
{/if}

{#if sessions.length === 0}
  <p class="mt-[16px] text-brand-muted">No sessions yet.</p>
{:else if sessionGroups.length === 0}
  <p class="mt-[16px] text-brand-muted">No sessions to display.</p>
{:else}
  {#each sessionGroups as group (group.status)}
    <!-- Group heading -->
    <div class="mt-[28px] mb-[10px] flex items-center gap-[10px]">
      {#if group.status === "lobby"}
        <span
          class="inline-block h-[8px] w-[8px] rounded-full"
          style="background: #8fb4ec;"
        ></span>
      {:else if group.status === "running"}
        <span
          class="inline-block h-[8px] w-[8px] rounded-full"
          style="background: #fb341c; animation: pulse 1.5s ease-in-out infinite;"
        ></span>
      {:else if group.status === "finished"}
        <span
          class="inline-block h-[8px] w-[8px] rounded-full"
          style="background: #6be597;"
        ></span>
      {:else}
        <span
          class="inline-block h-[8px] w-[8px] rounded-full"
          style="background: #d1d5db;"
        ></span>
      {/if}
      <h3
        class="text-[14px] font-bold uppercase tracking-wider"
        style="color: #9ea7b6;"
      >
        {group.label}
        <span class="ml-[6px] font-normal">({group.sessions.length})</span>
      </h3>
    </div>

    <div class="flex flex-col gap-[8px]">
      {#each group.sessions as session (session.id)}
        {@const cd = wsClient?.sessionCountdowns[session.id]}
        <div
          class="flex flex-wrap overflow-hidden rounded-[8px] bg-white"
          style={group.status === "cancelled" ? "opacity: 0.65;" : ""}
        >
          <!-- Join code -->
          <div
            class="flex w-[100px] shrink-0 flex-col justify-center py-[20px] pl-5 pr-[16px] sm:w-[120px] sm:pl-[32px]"
          >
            <div class="text-[11px] font-semibold text-brand-muted">ID</div>
            <p class="font-mono font-semibold text-brand-text">
              {session.join_code ?? "—"}
            </p>
          </div>

          <!-- Session name -->
          <div class="flex flex-1 flex-col justify-center py-[20px] pr-[16px]">
            <div class="text-[11px] font-semibold text-brand-muted">
              Session
            </div>
            <p class="font-semibold text-brand-text">{session.name}</p>
          </div>

          <!-- Players -->
          <div
            class="flex w-[90px] shrink-0 flex-col justify-center py-[20px] pr-[16px]"
          >
            <div class="text-[11px] font-semibold text-brand-muted">Players</div>
            <p class="font-semibold tabular-nums text-brand-text">
              {session.player_count ?? 0}
            </p>
          </div>

          <!-- Winner: only meaningful once the session has a final score -->
          {#if session.best_player}
            <div
              class="flex w-[150px] shrink-0 flex-col justify-center py-[20px] pr-[16px]"
            >
              <div class="text-[11px] font-semibold text-brand-muted">Winner</div>
              <p class="truncate font-semibold text-brand-text" title={session.best_player}>
                {session.best_player}
              </p>
              {#if session.best_score != null}
                <!-- Judges can drive a whole field negative; match the
                     Top Players convention and tint those red. A zero
                     stays neutral rather than reading as a win. -->
                <p
                  class="text-[12px] font-semibold tabular-nums"
                  style="color: {session.best_score < 0
                    ? '#e5484d'
                    : session.best_score > 0
                      ? '#2e9e5b'
                      : '#9ea7b6'};"
                >
                  {session.best_score} pts
                </p>
              {/if}
            </div>
          {/if}

          <!-- Started / Live ticker column -->
          <div
            class="flex w-[130px] shrink-0 flex-col justify-center py-[20px] pr-[16px] sm:w-[160px]"
          >
            {#if cd}
              <div class="text-[11px] font-semibold text-brand-muted">
                {cd.type === "lobby" ? "Starts in" : "Ends in"}
              </div>
              <p
                class="font-mono font-semibold tabular-nums"
                style="color: {cd.type === 'lobby' ? '#8fb4ec' : '#fb341c'};"
              >
                {fmtSecs(cd.secs)}
              </p>
            {:else}
              <div class="text-[11px] font-semibold text-brand-muted">
                Started
              </div>
              <!-- Date and time, because a busy project runs several sessions
                   a day and the date alone cannot tell them apart. Stacked
                   rather than joined: the column is 130px wide. -->
              <p class="text-sm text-brand-text">
                {formatDateUTC(session.created_at)}
              </p>
              <p class="text-[12px] tabular-nums text-brand-muted">
                {formatTimeUTC(session.created_at)}
              </p>
            {/if}
          </div>

          <!-- View / Join button -->
          {#if session.join_code && (group.status === "lobby" || group.status === "running")}
            <a
              href="/s/{session.join_code}"
              class="flex w-full shrink-0 flex-col items-center justify-center sm:w-[180px] px-[24px] py-[20px] text-center text-white transition-opacity hover:opacity-80"
              style="background: #36547f; border-left: 8px solid #fb341c;"
            >
              <span class="text-[11px] opacity-70">join</span>
              <span class="font-mono font-semibold">{session.join_code}</span>
            </a>
          {:else if session.join_code}
            <a
              href="/s/{session.join_code}"
              class="flex w-full shrink-0 items-center justify-center sm:w-[180px] px-[24px] py-[20px] text-center font-semibold text-white transition-opacity hover:opacity-80"
              style="background: {group.status === 'cancelled'
                ? '#9ea7b6'
                : '#36547f'}; border-left: 8px solid {group.status === 'finished'
                ? '#6be597'
                : '#9ea7b6'};"
            >
              View
            </a>
          {:else}
            <span
              class="flex w-full shrink-0 items-center justify-center sm:w-[180px] px-[24px] py-[20px] text-center font-semibold text-white"
              style="background: #9ea7b6; border-left: 8px solid #9ea7b6;"
            >
              Unavailable
            </span>
          {/if}
        </div>
      {/each}
    </div>
  {/each}
{/if}

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
</style>