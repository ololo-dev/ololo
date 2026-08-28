<script lang="ts">
  // Spotlight for sessions you can still act on — running (watch) and lobby
  // (join before it starts) — shown above the Top Players / Sessions tabs so a
  // visitor lands on proof the project is live and can jump straight in. The
  // same sessions still appear in the Sessions tab; this is a shortcut.

  import type { Session } from "$lib/api";
  import type { WsProjectClient } from "$lib/ws-project.svelte";
  import { formatHms as fmtSecs } from "$lib/format";
  import CodeBlock from "$lib/components/CodeBlock.svelte";

  let {
    sessions,
    wsClient,
  }: {
    sessions: Session[];
    wsClient: WsProjectClient | null;
  } = $props();

  function players(session: Session): number {
    return session.player_count ?? 0;
  }
</script>

{#if sessions.length > 0}
  <section class="mt-[48px]" aria-label="Live sessions" data-testid="live-sessions">
    <div class="mb-[10px] flex items-center gap-[10px]">
      <span class="live-dot inline-block h-[8px] w-[8px] rounded-full" style="background: #fb341c;"
      ></span>
      <h3 class="font-heading text-[14px] font-bold uppercase tracking-wider" style="color: #9ea7b6;">
        Live now
        <span class="ml-[6px] font-normal">({sessions.length})</span>
      </h3>
    </div>

    <ul class="grid gap-[8px] sm:grid-cols-2 lg:grid-cols-3">
      {#each sessions as session (session.id)}
        {@const cd = wsClient?.sessionCountdowns[session.id]}
        {@const lobby = session.status === "lobby"}
        {@const accent = lobby ? "#8fb4ec" : "#fb341c"}
        <li
          class="flex flex-col gap-[16px] overflow-hidden rounded-[8px] bg-white px-[24px] py-[20px]"
          style="border-left: 8px solid {accent};"
          data-testid="live-session"
        >
          <!-- Status + player count -->
          <div class="flex items-center justify-between gap-[12px]">
            <span
              class="inline-flex items-center gap-[6px] rounded-full px-[10px] py-[3px] text-[11px] font-bold uppercase tracking-wide"
              style="background: {lobby ? '#e2ecfc' : '#ffe8e5'}; color: {lobby
                ? '#4a7fd4'
                : '#c92912'};"
            >
              <span
                class="inline-block h-[6px] w-[6px] rounded-full {lobby ? '' : 'live-dot'}"
                style="background: {accent};"
              ></span>
              {lobby ? "Lobby" : "Running"}
            </span>
            <span class="shrink-0 text-[12px] text-brand-muted">
              {players(session)}
              {players(session) === 1 ? "player" : "players"}
            </span>
          </div>

          <!-- Name -->
          <div class="min-w-0">
            <p class="truncate font-semibold text-brand-text" title={session.name}>
              {session.name}
            </p>
          </div>

          <!-- Countdown: server-driven, so it only appears once a tick lands -->
          {#if cd}
            <div>
              <div class="text-[11px] font-semibold text-brand-muted">
                {cd.type === "lobby" ? "Starts in" : "Ends in"}
              </div>
              <p class="font-mono text-[18px] font-semibold tabular-nums" style="color: {accent};">
                {fmtSecs(cd.secs)}
              </p>
            </div>
          {/if}

          <!--
            Joining only ever happens from a terminal — there is no browser or
            mobile path in — so the copyable command is the primary action and
            the link is the secondary "follow along" affordance.
          -->
          {#if session.join_code}
            {@const code = session.join_code}
            <div class="mt-auto">
              <CodeBlock code="ololo join {code}" />
              <a
                href="/s/{code}"
                class="mt-[10px] inline-block text-[13px] font-semibold text-brand-blue hover:underline"
              >
                Watch live →
              </a>
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  </section>
{/if}

<style>
  .live-dot {
    animation: pulse 1.5s ease-in-out infinite;
  }

  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.3;
    }
  }

  /* Respect users who ask the OS to reduce motion. */
  @media (prefers-reduced-motion: reduce) {
    .live-dot {
      animation: none;
    }
  }
</style>
