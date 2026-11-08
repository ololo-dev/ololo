<script lang="ts">
  import { browser } from "$app/environment";
  import type {
    ActivityEvent,
    LeaderboardEntry,
    MemberInfo,
    PlayerCompletionStatus,
    ScoreHistoryPoint,
    SessionHealthPayload,
  } from "$lib/types/arena";
  import type { SessionLinks } from "$lib/api";
  import { slideStops, stopBy, type ScreenSlide, type SlideTarget } from "$lib/session-links";
  import { ikAvatar } from "$lib/imagekit";
  import type { HealthIndicator } from "$lib/session-health";
  import ScoreChart from "./ScoreChart.svelte";
  import ScreenLeaderboard from "./ScreenLeaderboard.svelte";
  import ScreenTicker from "./ScreenTicker.svelte";
  import ScreenCombinedLeaderboard from "./ScreenCombinedLeaderboard.svelte";

  type Phase = "lobby" | "active" | "paused" | "finished";
  type Theme = "dark" | "light";

  interface ScreenRow {
    player_id: string;
    display_name: string;
    avatar_url?: string | null;
    agent_display_name?: string | null;
    completion_status?: PlayerCompletionStatus | null;
    total_points: number;
    health?: HealthIndicator | null;
  }

  let {
    phase,
    projectName,
    joinCode,
    timerDigits,
    participants,
    entries,
    leaderboard,
    reportMembers,
    scoreHistory,
    health = null,
    events,
    avatarByPlayerId = new Map<string, string | null>(),
    judgingPending = false,
    resultSummary = null,
    exitHref,
    onExit = null,
    links = null,
    slide = "session",
    onSlide = null,
  }: {
    phase: Phase;
    projectName: string | null | undefined;
    joinCode: string;
    /** HH, MM, SS — the countdown to the start (lobby) or to the end. */
    timerDigits: [string, string, string];
    participants: MemberInfo[];
    /** Leaderboard rows as the board shows them: live standings while the
     * game is on, the report's once it is over. */
    entries: ScreenRow[];
    /** The chart's participant lines (its own source while live). */
    leaderboard: LeaderboardEntry[];
    reportMembers: MemberInfo[];
    scoreHistory: ScoreHistoryPoint[];
    health?: SessionHealthPayload | null;
    events: ActivityEvent[];
    avatarByPlayerId?: Map<string, string | null>;
    judgingPending?: boolean;
    resultSummary?: string | null;
    /** Back to the ordinary session page. */
    exitHref: string;
    /** Leave without a navigation — the page swaps the board back for the
     * dashboard itself (and drops the browser's fullscreen). */
    onExit?: (() => void) | null;
    /** The sessions this one is linked with — rooms of one event, running
     * side by side: the slider flips the board between them and to the
     * combined standings (live while they play, final once every one has
     * ended). Null (or a group of one) leaves the board exactly as it was. */
    links?: SessionLinks | null;
    slide?: ScreenSlide;
    /** The page moves the board: to another linked session, or to "overall". */
    onSlide?: ((target: SlideTarget) => void) | null;
  } = $props();

  // ── Theme ──────────────────────────────────────────────────────────────
  // Dark reads best through a compressed video stream; light suits a bright
  // room with a projector. The viewer's pick is remembered in this browser
  // (a per-viewer convenience — never shared state).
  const THEME_KEY = "ololo.screen.theme";
  function readTheme(): Theme {
    try {
      return localStorage.getItem(THEME_KEY) === "light" ? "light" : "dark";
    } catch {
      return "dark";
    }
  }
  let theme = $state<Theme>(browser ? readTheme() : "dark");
  function toggleTheme() {
    theme = theme === "dark" ? "light" : "dark";
    try {
      localStorage.setItem(THEME_KEY, theme);
    } catch {
      // Storage unavailable: the choice lasts this visit.
    }
  }
  // The document behind the board (overscroll, the moment before paint)
  // follows the theme too — see `html.screen-frame` in app.css.
  $effect(() => {
    if (!browser) return;
    document.documentElement.dataset.screenTheme = theme;
    return () => {
      delete document.documentElement.dataset.screenTheme;
    };
  });

  const PALETTES: Record<Theme, Record<string, string>> = {
    dark: {
      "--s-bg": "#0b1628",
      "--s-card": "rgba(255,255,255,0.05)",
      "--s-border": "rgba(255,255,255,0.08)",
      "--s-text": "#ffffff",
      "--s-head": "#8fb4ec",
      "--s-sub": "#b8c7dc",
      "--s-dim": "#5b6b86",
      "--s-pos": "#6be597",
      "--s-neg": "#ff6b5c",
      "--s-chip": "rgba(255,255,255,0.08)",
      "--s-chip-border": "rgba(255,255,255,0.12)",
      "--s-avatar": "#36547f",
      "--s-avatar-text": "#ffffff",
      "--s-timer": "rgba(255,255,255,0.1)",
      "--s-timer-text": "#ffffff",
      "--s-top": "rgba(245,166,35,0.10)",
      "--s-hover": "rgba(255,255,255,0.1)",
      "--s-join": "#6be597",
    },
    light: {
      "--s-bg": "#f4f8fe",
      "--s-card": "#ffffff",
      "--s-border": "#e3ebf6",
      "--s-text": "#363636",
      "--s-head": "#36547f",
      "--s-sub": "#5b6b86",
      "--s-dim": "#9ea7b6",
      "--s-pos": "#3aa568",
      "--s-neg": "#ef4444",
      "--s-chip": "#eef3fb",
      "--s-chip-border": "#dce9fc",
      "--s-avatar": "#8fb4ec",
      "--s-avatar-text": "#ffffff",
      "--s-timer": "#36547f",
      "--s-timer-text": "#ffffff",
      "--s-top": "rgba(245,166,35,0.14)",
      "--s-hover": "rgba(54,84,127,0.08)",
      "--s-join": "#0269fb",
    },
  };
  const paletteStyle = $derived(
    Object.entries(PALETTES[theme])
      .map(([k, v]) => `${k}: ${v}`)
      .join("; "),
  );

  // The chart fills its card: measure the card body and hand the chart
  // the height, minus the heading above it.
  let chartBodyHeight = $state(0);
  const chartHeight = $derived(Math.max(200, chartBodyHeight - 8));

  const running = $derived(phase === "active" || phase === "paused");

  // The slider: one stop per linked session, in the order they were linked,
  // and — once every one of them has ended — a last stop for the combined
  // standings. A session linked to nothing has no slider.
  const stops = $derived(slideStops(links, slide));
  const hasSlider = $derived(stops.length > 0);
  const overall = $derived(slide === "overall" && stops.length > 0);
  function slideBy(step: number) {
    const next = stopBy(stops, step);
    if (next) onSlide?.(next.target);
  }
  const statusLabel = $derived(
    overall
      ? links?.all_finished
        ? "Final standings"
        : "Overall · live"
      : phase === "active"
      ? "Live"
      : phase === "paused"
        ? "Paused"
        : phase === "lobby"
          ? "Starts in"
          : judgingPending
            ? "Judging"
            : "Final results",
  );

  // `f` toggles the browser's own fullscreen — the screen is built for a
  // projector or a shared window, where the address bar is noise. `t`
  // flips the theme.
  function onKey(e: KeyboardEvent) {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const target = e.target as HTMLElement | null;
    if (target && ["INPUT", "TEXTAREA"].includes(target.tagName)) return;
    if (hasSlider && e.key === "ArrowRight") {
      slideBy(1);
      return;
    }
    if (hasSlider && e.key === "ArrowLeft") {
      slideBy(-1);
      return;
    }
    if (e.key === "t") {
      toggleTheme();
      return;
    }
    if (e.key !== "f" || !browser || !document.fullscreenEnabled) return;
    if (document.fullscreenElement) void document.exitFullscreen();
    else void document.documentElement.requestFullscreen().catch(() => {});
  }
</script>

<!-- `f` toggles the browser's own fullscreen, `t` flips the theme; with
     linked sessions, the arrow keys move the slider. -->
<svelte:window onkeydown={onKey} />

<!-- The session as a room sees it: one screen, no site chrome, no scroll —
     the thing a host shares on a call or throws on a projector. Top: phase,
     project, the join command, the clock. Middle: the chart and the board
     (score and code health per player). Bottom: the last few moves. Every
     colour is a palette variable, so the two themes are one markup. -->
<div
  class="grid h-[100dvh] w-full gap-[16px] px-[28px] py-[20px]"
  style="grid-template-rows: auto {hasSlider ? 'auto ' : ''}minmax(0, 1fr){overall ? '' : ' auto'}; background: var(--s-bg); color: var(--s-text); {paletteStyle};"
  data-testid="session-screen"
  data-theme={theme}
  data-slide={slide}
>
  <!-- Top strip -->
  <div class="flex items-center justify-between gap-[24px]">
    <div class="flex min-w-0 items-center gap-[14px]">
      {#if phase === "active"}
        <span
          class="live-dot inline-block h-[12px] w-[12px] shrink-0 rounded-full"
          style="background: #fb341c;"
          aria-hidden="true"
        ></span>
      {/if}
      <span
        class="shrink-0 text-[clamp(12px,1vw,16px)] font-semibold uppercase tracking-[0.18em]"
        style="color: var(--s-head);"
        data-testid="screen-status"
      >{statusLabel}</span>
      {#if projectName}
        <span class="truncate font-heading text-[clamp(20px,2vw,36px)] font-bold">{projectName}</span>
      {/if}
    </div>

    {#if phase !== "finished"}
      <!-- The one thing a newcomer needs from the screen. -->
      <div
        class="flex shrink-0 items-center gap-[10px] rounded-[10px] px-[18px] py-[8px] font-mono text-[clamp(16px,1.8vw,32px)] font-semibold"
        style="background: var(--s-chip); border: 1px solid var(--s-chip-border);"
        data-testid="screen-join"
      >
        <span style="color: var(--s-head);">$</span>
        <span>ololo join <span style="color: var(--s-join);">{joinCode}</span></span>
      </div>
    {/if}

    <div class="flex shrink-0 items-center gap-[18px]">
      {#if phase !== "finished"}
        <div class="flex items-center gap-[8px]" aria-label="Time left" data-testid="screen-timer">
          {#each timerDigits as digit, i (i)}
            <span
              class="inline-flex h-[1.9em] w-[1.9em] items-center justify-center rounded-[8px] text-[clamp(18px,2.2vw,40px)] font-bold tabular-nums"
              style="background: var(--s-timer); color: var(--s-timer-text);"
            >{digit}</span>
            {#if i < 2}
              <span class="text-[clamp(18px,2.2vw,40px)] font-bold" style="opacity: 0.4;">:</span>
            {/if}
          {/each}
        </div>
      {/if}
      <!-- A finished session has no room (and no roster) any more: the
           report's standings say who played. -->
      <span class="text-[clamp(12px,1vw,16px)] font-semibold" style="color: var(--s-dim);">
        {#if phase === "lobby"}
          {participants.length} in lobby
        {:else if running}
          {participants.length} playing
        {:else}
          {entries.length} played
        {/if}
      </span>
      <button
        type="button"
        onclick={toggleTheme}
        class="theme-button inline-flex h-[30px] w-[30px] items-center justify-center rounded-[6px] transition-colors"
        style="color: var(--s-dim);"
        title={theme === "dark" ? "Light theme (T)" : "Dark theme (T)"}
        aria-label={theme === "dark" ? "Switch to the light theme" : "Switch to the dark theme"}
        aria-pressed={theme === "light"}
        data-testid="screen-theme"
      >
        {#if theme === "dark"}
          <!-- sun -->
          <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="4"></circle><path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M4.93 19.07l1.41-1.41M17.66 6.34l1.41-1.41"></path></svg>
        {:else}
          <!-- moon -->
          <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"></path></svg>
        {/if}
      </button>
      <a
        href={exitHref}
        onclick={(e) => {
          if (!onExit) return;
          e.preventDefault();
          onExit();
        }}
        class="theme-button rounded-[6px] px-[10px] py-[4px] text-[clamp(11px,0.9vw,14px)] font-semibold transition-colors"
        style="color: var(--s-dim);"
        title="Back to the session page (Esc or F leave the browser's fullscreen)"
      >Exit</a>
    </div>
  </div>

  {#if hasSlider}
    <!-- The slider between linked sessions: one stop per session, the
         combined standings last once every session has ended. Arrows and
         the ← → keys walk it; a stop is a door to that session's board. -->
    <nav
      class="flex items-center justify-center gap-[10px]"
      aria-label="Linked sessions"
      data-testid="screen-slider"
    >
      <button
        type="button"
        class="slide-arrow inline-flex h-[32px] w-[32px] shrink-0 items-center justify-center rounded-full"
        style="color: var(--s-dim);"
        onclick={() => slideBy(-1)}
        aria-label="Previous session"
        data-testid="screen-slider-prev"
      >‹</button>
      <ol class="flex min-w-0 flex-wrap items-center justify-center gap-[8px]">
        {#each stops as stop (stop.key)}
          <li>
            <button
              type="button"
              class="slide-stop max-w-[22vw] truncate rounded-full px-[14px] py-[5px] text-[clamp(12px,1vw,16px)] font-semibold"
              style={stop.current
                ? "background: var(--s-head); color: var(--s-bg);"
                : "background: var(--s-chip); border: 1px solid var(--s-chip-border); color: var(--s-sub);"}
              aria-current={stop.current ? "true" : undefined}
              onclick={() => {
                if (!stop.current) onSlide?.(stop.target);
              }}
              title={stop.key === "overall" ? "Standings across every linked session" : `ololo join ${stop.key}`}
              data-testid="screen-slider-stop"
              data-key={stop.key}
            >{stop.label}</button>
          </li>
        {/each}
      </ol>
      <button
        type="button"
        class="slide-arrow inline-flex h-[32px] w-[32px] shrink-0 items-center justify-center rounded-full"
        style="color: var(--s-dim);"
        onclick={() => slideBy(1)}
        aria-label="Next session"
        data-testid="screen-slider-next"
      >›</button>
    </nav>
  {/if}

  <!-- Middle -->
  {#if overall && links}
    <!-- The whole event: every room's players in one list by points — live
         while the rooms play, the final word once every one has ended. -->
    <div
      class="flex min-h-0 flex-col overflow-hidden rounded-[12px] px-[16px] py-[18px]"
      style="background: var(--s-card); border: 1px solid var(--s-border);"
      data-testid="screen-overall"
    >
      <h2 class="mb-[10px] shrink-0 px-[12px] font-heading text-[clamp(14px,1.2vw,20px)] font-bold" style="color: var(--s-head);">
        {links.all_finished ? "Final standings" : "Overall standings"} · {links.sessions.length} sessions
        {#if !links.all_finished}
          <span class="ml-[8px] text-[clamp(11px,0.9vw,15px)] font-semibold uppercase tracking-[0.12em]" style="color: var(--s-dim);" data-testid="screen-overall-live">live</span>
        {/if}
      </h2>
      <div class="min-h-0 flex-grow overflow-hidden">
        <ScreenCombinedLeaderboard {links} />
      </div>
    </div>
  {:else if phase === "lobby"}
    <div class="flex min-h-0 flex-col items-center justify-center gap-[28px]">
      <p class="text-[clamp(16px,1.4vw,24px)]" style="color: var(--s-head);">Join from your terminal</p>
      <div
        class="rounded-[16px] px-[36px] py-[20px] font-mono text-[clamp(32px,5vw,96px)] font-bold"
        style="background: var(--s-card); border: 1px solid var(--s-border);"
      >
        ololo join <span style="color: var(--s-join);">{joinCode}</span>
      </div>
      <!-- Faces as they arrive: the lobby's whole story. -->
      <div class="flex max-w-[80vw] flex-wrap justify-center gap-[12px]" data-testid="screen-roster">
        {#each participants as m (m.user_id)}
          <span
            class="inline-flex items-center gap-[10px] rounded-full py-[6px] pl-[6px] pr-[16px] text-[clamp(14px,1.3vw,22px)] font-semibold"
            style="background: var(--s-chip);"
          >
            {#if m.avatar_url}
              <img src={ikAvatar(m.avatar_url, 36)} alt="" class="h-[36px] w-[36px] rounded-full object-cover" />
            {:else}
              <span
                class="inline-flex h-[36px] w-[36px] items-center justify-center rounded-full text-[15px] font-bold"
                style="background: var(--s-avatar); color: var(--s-avatar-text);"
              >{m.display_name.charAt(0).toUpperCase()}</span>
            {/if}
            {m.display_name}
          </span>
        {/each}
      </div>
    </div>
  {:else}
    <div class="grid min-h-0 gap-[16px]" style="grid-template-columns: minmax(0, 1fr) clamp(320px, 28vw, 520px);">
      <div
        class="flex min-h-0 flex-col rounded-[12px] px-[24px] py-[18px]"
        style="background: var(--s-card); border: 1px solid var(--s-border);"
      >
        <h2 class="mb-[10px] shrink-0 font-heading text-[clamp(14px,1.2vw,20px)] font-bold" style="color: var(--s-head);">
          Score over time
        </h2>
        <div class="min-h-0 flex-grow" bind:clientHeight={chartBodyHeight}>
          <ScoreChart
            {phase}
            {leaderboard}
            {reportMembers}
            {scoreHistory}
            {health}
            height={chartHeight}
            dark={theme === "dark"}
          />
        </div>
      </div>
      <div
        class="flex min-h-0 flex-col overflow-hidden rounded-[12px] px-[16px] py-[18px]"
        style="background: var(--s-card); border: 1px solid var(--s-border);"
      >
        <h2 class="mb-[10px] shrink-0 px-[12px] font-heading text-[clamp(14px,1.2vw,20px)] font-bold" style="color: var(--s-head);">
          {running ? "Leaderboard" : "Final results"}
        </h2>
        {#if !running && resultSummary}
          <p class="mb-[10px] shrink-0 px-[12px] text-[clamp(13px,1.1vw,18px)]" style="color: var(--s-sub);">{resultSummary}</p>
        {/if}
        <div class="min-h-0 flex-grow overflow-hidden">
          <ScreenLeaderboard
            {entries}
            emptyMessage={running ? "Waiting for first results…" : "No results recorded."}
          />
        </div>
      </div>
    </div>
  {/if}

  <!-- Bottom strip — none on the Overall slide: the last moves and the
       final line belong to one room, not to the event. -->
  {#if overall}
    <!-- nothing -->
  {:else if phase === "lobby"}
    <p class="text-center text-[clamp(12px,1vw,16px)]" style="color: var(--s-dim);">
      ololo.dev/s/{joinCode}
    </p>
  {:else}
    <!-- The last few moves — live, and the closing ones once it is over. -->
    <div
      class="rounded-[12px] px-[24px] py-[14px]"
      style="background: var(--s-card); border: 1px solid var(--s-border);"
    >
      <ScreenTicker {events} {avatarByPlayerId} limit={4} />
    </div>
  {/if}
</div>

<style>
  .slide-arrow:hover,
  .slide-stop:hover {
    background: var(--s-hover);
  }
  .live-dot {
    animation: screen-pulse 1.5s ease-in-out infinite;
  }
  @keyframes screen-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.3; }
  }
  .theme-button:hover {
    background: var(--s-hover);
  }
</style>
