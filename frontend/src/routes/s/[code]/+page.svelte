<script lang="ts">
  import { browser } from "$app/environment";
  import { onMount, onDestroy, untrack } from "svelte";
  import type { LeaderboardEntry, MemberInfo, ScoreHistoryPoint, ActivityEvent } from "$lib/types/arena";
  import type { PageData } from "./$types";
  import type { SessionReportResponse } from "$lib/api";
  import { patchSession, getSessionPlayerStats, getSessionReport } from "$lib/api";
  import type { SessionPlayerStatsResponse } from "$lib/api";
  import SessionPlayerStatsBlock from "$lib/components/session/SessionPlayerStatsBlock.svelte";
  import { invalidateAll } from "$app/navigation";
  import { notify } from "$lib/notifications.svelte";
  import { WsSessionClient } from "$lib/ws-session.svelte";
  import SessionCongratsModal from "$lib/components/session/SessionCongratsModal.svelte";
  import { sessionStateToUiPhase } from "$lib/session-phase";
  import { getToken, initAuth } from "$lib/auth";
  import SessionLobby from "$lib/components/session/SessionLobby.svelte";
  import { mergeLeaderboardEntries } from "$lib/session-leaderboard";
  import ScoreChart from "$lib/components/session/ScoreChart.svelte";
  import LeaderboardList from "$lib/components/session/LeaderboardList.svelte";
  import ActivityLog from "$lib/components/session/ActivityLog.svelte";
  import SessionCostsPanel from "$lib/components/session/SessionCostsPanel.svelte";
  import TopBar from "$lib/components/session/TopBar.svelte";
  import SessionHeaderCard from "$lib/components/session/SessionHeaderCard.svelte";
  import SessionCampaignCard from "$lib/components/session/SessionCampaignCard.svelte";
  import SessionReplay from "$lib/components/session/SessionReplay.svelte";
  import { createReplay } from "$lib/replay.svelte";
  import { countdownDigits } from "$lib/format";

  let { data }: { data: PageData } = $props();

  // ── Owner/admin control gating ───────────────────────────────────────────
  const isOwner = $derived(data.user?.id != null && data.user.id === data.session.owner_id);
  const isAdmin = $derived(data.isAdmin ?? false);
  const canControl = $derived(isOwner || isAdmin);

  // ponytail: optimistic flag — disables all control buttons during any in-flight patch
  let controlBusy = $state(false);

  async function setSessionStatus(next: "paused" | "running" | "cancelled"): Promise<void> {
    if (controlBusy) return;
    controlBusy = true;
    try {
      await patchSession(data.session.id, { status: next }, { fetch });
      notify.success(`Session ${next}.`);
      await invalidateAll();
    } catch (err) {
      notify.error(`Failed to ${next} session: ${err instanceof Error ? err.message : String(err)}`);
    } finally {
      controlBusy = false;
    }
  }

  function onCancelClick(): void {
    if (!window.confirm("Cancel session? This ends the session for all players.")) return;
    void setSessionStatus("cancelled");
  }

  // ── Phase ─────────────────────────────────────────────────────────────────

  type Phase = "lobby" | "active" | "paused" | "finished";

  let phase = $state<Phase>(untrack(() => sessionStateToUiPhase(data.session.state)));

  // Browser-tab title tracks the phase: a finished session is not a "live
  // session", and with several session tabs open the join code + project name
  // are what tells them apart (audit UI-M5).
  const pageTitle = $derived.by(() => {
    const what = data.session.project_name
      ? `${data.session.join_code} · ${data.session.project_name}`
      : data.session.join_code;
    const state =
      phase === "finished"
        ? "Results"
        : phase === "lobby"
          ? "Lobby"
          : phase === "paused"
            ? "Paused"
            : "Live";
    return `${state}: ${what} — ololo.dev`;
  });
  let countdownSecs = $state<number | null>(null);
  let runningCountdownSecs = $state<number | null>(null);
  let leaderboard = $state<LeaderboardEntry[]>([]);
  let participants = $state<MemberInfo[]>([]);
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  let scoreHistory = $state<ScoreHistoryPoint[]>([]);
  let sessionReport = $state<SessionReportResponse | null>(untrack(() => data.report ?? null));
  let userPlayers = $state<import("$lib/types/arena").PlayerSummary[]>([]);
  let activityLog = $state<ActivityEvent[]>([]);
  let sessionStartedAt = $state<Date | null>(null);
  let playerStats = $state<SessionPlayerStatsResponse | null>(null);

  // Statistics block data: the block renders only for a finished session
  // (final probe/task/token counts are meaningless mid-run), so fetch only
  // once the phase reaches "finished" rather than on every load.
  let statsFetched = $state(false);
  $effect(() => {
    if (!browser || phase !== "finished" || statsFetched) return;
    statsFetched = true;
    getSessionPlayerStats(data.session.id)
      .then((s) => (playerStats = s))
      .catch(() => {});
  });

  // ── WS client ─────────────────────────────────────────────────────────────

  let wsClient = $state<WsSessionClient | null>(null);

  // Session-end notification for observers: shown when the end arrives while
  // the dashboard is open (a page opened after the fact gets no banner).
  let showSessionEndBanner = $state(false);
  let endedCancelled = $state(false);
  let cancelDetail = $state('Session was cancelled — no more probes will run.');

  // ── End-of-session celebration ────────────────────────────────────────────
  // When the finish arrives live (not on a later visit), congratulate — but
  // only once the judges are done: the timer stopping flips the status while
  // judge runs may still be landing verdicts, and a popup naming a winner
  // before the last verdict would celebrate a standings that can still move.
  // The server pushes `session_settled` over the WS the moment the last owed
  // judge run lands (the same signal AP awarding waits on); on it we fetch
  // the report once for the final numbers — no interval polling.
  let celebration = $state<'idle' | 'waiting' | 'shown' | 'dismissed'>('idle');
  let celebrationTimer: ReturnType<typeof setTimeout> | null = null;

  function finalizeCelebration() {
    if (celebrationTimer) {
      clearTimeout(celebrationTimer);
      celebrationTimer = null;
    }
    getSessionReport(data.session.id)
      .then((r) => (sessionReport = r))
      .finally(() => (celebration = 'shown'));
  }

  $effect(() => {
    if (celebration === 'waiting' && wsClient?.settled) finalizeCelebration();
  });

  // "Complete" is only honest once the last owed judge run is terminal
  // (scored or failed). Until then the header badge says "Judging": the
  // server-loaded report carries judges_pending, and the live settle frame
  // (or the report refetch it triggers) flips it. A missing report defaults
  // to complete rather than sticking on an amber badge forever; cancelled
  // sessions never judged, so they are complete outright.
  const judgingPending = $derived(
    !endedCancelled &&
      !wsClient?.settled &&
      (sessionReport?.judges_pending ?? 0) > 0,
  );

  // ── Copy command ──────────────────────────────────────────────────────────

  let copied = $state(false);
  let copyTimeout: ReturnType<typeof setTimeout> | null = null;

  function copyCommand() {
    if (!browser) return;
    navigator.clipboard.writeText(`ololo join ${data.session.join_code}`).then(() => {
      copied = true;
      if (copyTimeout) clearTimeout(copyTimeout);
      copyTimeout = setTimeout(() => { copied = false; }, 2000);
    });
  }

  // Sync WS client state to local reactive state.
  // Phase only advances (lobby → active → finished); never regresses.
  $effect(() => {
    if (!wsClient) return;
    countdownSecs = wsClient.countdownSecs;
    runningCountdownSecs = wsClient.runningCountdownSecs;
    leaderboard = wsClient.leaderboard;
    participants = wsClient.participants;
    scoreHistory = wsClient.scoreHistory;
    userPlayers = wsClient.userPlayers;
    activityLog = wsClient.activityLog;
    sessionStartedAt = wsClient.startedAt;

    const wp = wsClient.phase;
    if (wp === "finished") {
      if (phase !== "finished") {
        showSessionEndBanner = true;
        // A completed (not cancelled) run earns a congratulations popup once
        // the judge queue drains. The settled frame normally arrives over the
        // WS; the timer is a one-shot safety net for a dropped connection.
        if (!wsClient.endedCancelled && celebration === 'idle') {
          celebration = 'waiting';
          celebrationTimer = setTimeout(() => finalizeCelebration(), 10 * 60 * 1000);
        }
      }
      endedCancelled = wsClient.endedCancelled;
      if (wsClient.cancelReason === 'idle_timeout') {
        cancelDetail = 'Session ended automatically — no players were connected.';
      } else if (wsClient.cancelReason === 'user') {
        cancelDetail = wsClient.cancelledBy
          ? `Session was cancelled by ${wsClient.cancelledBy}.`
          : 'Session was cancelled.';
      }
      phase = "finished";
    } else if (wp === "paused" && phase === "active") {
      phase = "paused";
    } else if (wp === "active" && phase === "paused") {
      phase = "active";
    } else if (wp === "active" && phase !== "finished") {
      phase = "active";
    }
  });

  const timerDigits = $derived(
    phase === "lobby"
      ? countdownDigits(countdownSecs ?? 0)
      : countdownDigits(runningCountdownSecs ?? 0),
  );

  const ownPlayerIds = $derived(new Set(userPlayers.map((p) => p.player_id)));

  /**
   * Map user_id → player_id for own players.
   * Needed to match unscored leaderboard entries, which use participant.user_id
   * as their synthetic player_id instead of the actual player UUID.
   */
  const ownUserIdToPlayerId = $derived(
    new Map(
      userPlayers
        .filter((p) => p.user_id !== null)
        .map((p) => [p.user_id as string, p.player_id]),
    ),
  );

  /**
   * Reverse map: player_id → user_id for own players.
   * Needed by mergedLeaderboardWithInfo: after the player_id normalisation, entries
   * carry the real player UUID, but participants[] is keyed by user_id.
   */
  const ownPlayerIdToUserId = $derived(
    new Map(
      userPlayers
        .filter((p) => p.user_id !== null)
        .map((p) => [p.player_id, p.user_id as string]),
    ),
  );

  // ── Merged leaderboard ────────────────────────────────────────────────────

  const mergedLeaderboard = $derived(
    mergeLeaderboardEntries({ leaderboard, participants, userPlayers }),
  );

  const celebrationEntries = $derived(
    sessionReport?.leaderboard?.length ? sessionReport.leaderboard : mergedLeaderboard,
  );

  // player_id → avatar_url for ActivityLog. Activity events carry the player
  // UUID; participants[] is keyed by user_id. Translate via the reverse map
  // before looking up the avatar. Anonymous participants (no user link) fall
  // back to matching by player_id directly against participants.
  // Finished sessions have no live WS, so seed from the report's players
  // (keyed by player_id directly); live WS data overrides when present.
  const avatarByPlayerId = $derived.by<Map<string, string | null>>(() => {
    const map = new Map<string, string | null>();
    for (const p of sessionReport?.players ?? []) {
      map.set(p.player_id, p.avatar_url);
    }
    for (const m of participants) {
      if (m.player_id) map.set(m.player_id, m.avatar_url ?? null);
    }
    for (const p of userPlayers) {
      if (map.has(p.player_id)) continue;
      const userId = p.user_id;
      const member = userId
        ? participants.find((m) => m.user_id === userId)
        : participants.find((m) => m.user_id === p.player_id);
      map.set(p.player_id, member?.avatar_url ?? map.get(p.player_id) ?? null);
    }
    return map;
  });

  // ── Leaderboard helpers ───────────────────────────────────────────────────

  const mergedLeaderboardWithInfo = $derived(
    mergedLeaderboard.map((entry) => {
      // Entries for own players carry the real player UUID as player_id, but
      // participants[] is keyed by user_id. Translate before the lookup.
      const lookupId = ownPlayerIdToUserId.get(entry.player_id) ?? entry.player_id;
      const member =
        participants.find((p) => p.player_id === entry.player_id) ??
        participants.find((p) => p.user_id === lookupId);
      return {
        ...entry,
        avatar_url: member?.avatar_url ?? null,
        fingerprint: member?.fingerprint ?? null,
        username: member?.username ?? null,
        // Absent on list paths / pre-upgrade servers — the row renders no badge.
        completion_status: member?.completion_status ?? null,
      };
    }),
  );

  // ── Activity log ──────────────────────────────────────────────────────────

  // The report REST DTO carries the kind as `event_kind`; ActivityLog renders
  // on the WS `kind` field, so normalize (and drop unknown kinds) here.
  const activityEvents = $derived<ActivityEvent[]>(
    sessionReport && sessionReport.activity_events
      ? sessionReport.activity_events
          .filter(
            (e): e is typeof e & { event_kind: ActivityEvent["kind"] } =>
              e.event_kind === "task_started" ||
              e.event_kind === "task_scored" ||
              e.event_kind === "artifact_received" ||
              e.event_kind === "similarity",
          )
          .map((e) => ({
            kind: e.event_kind,
            player_id: e.player_id,
            player_display_name: e.player_display_name,
            task_id: e.task_id,
            task_ordinal: e.task_ordinal,
            task_title: e.task_title,
            judge_name: e.judge_name,
            point_delta: e.point_delta,
            detail: e.detail ?? null,
            timestamp: e.timestamp,
            version: e.version,
          }))
      : activityLog,
  );

  // The session's `finished_at` is stamped when the timer stops, but judging
  // and final scoring events keep landing for a couple of minutes after —
  // the header's date range used to end before events visible in the feed
  // below it (audit UI-M5). Extend the displayed end to the last event.
  const effectiveFinishedAt = $derived.by<string | null>(() => {
    const finished = data.session.finished_at ?? null;
    let last = finished;
    for (const e of activityEvents) {
      if (last === null || e.timestamp > last) last = e.timestamp;
    }
    return last;
  });

  // Session start reference for elapsed-time display. Prefer the WS-provided
  // startedAt; fall back to the earliest activity event timestamp (finished
  // sessions have no live WS).
  const activityStartRef = $derived.by<Date | null>(() => {
    if (sessionStartedAt) return sessionStartedAt;
    const evs = activityEvents;
    if (evs.length === 0) return null;
    let earliest = evs[0].timestamp;
    for (const e of evs) if (e.timestamp < earliest) earliest = e.timestamp;
    return new Date(earliest);
  });

  // ── Report-derived data (finished sessions) ──────────────────────────────
  // Members for buildChart — sourced from the report leaderboard. For finished
  // sessions the WS snapshot may also seed participants[], but the report
  // leaderboard is the authoritative source here regardless of phase.
  // player_id → report player info (avatar/username) for finished-session views.
  const reportPlayerById = $derived(
    new Map((sessionReport?.players ?? []).map((p) => [p.player_id, p])),
  );

  // Score chart timeseries: live WS wins; finished sessions (no WS) fall back
  // to the history embedded in the report.
  const effectiveScoreHistory = $derived<ScoreHistoryPoint[]>(
    scoreHistory.length > 0 ? scoreHistory : (sessionReport?.score_history ?? []),
  );

  // ── Replay (finished sessions) ────────────────────────────────────────────
  // A playhead sweeps session time; the score chart draws in and the activity
  // feed reveals events up to it, at a chosen speed. Engine is shared across
  // the dashboard and player pages (createReplay); reveal flips on first
  // play/scrub — until then the report shows everything as normal.
  const maxReplayT = $derived.by(() => {
    let m = 0;
    for (const p of effectiveScoreHistory) if (p.t > m) m = p.t;
    const start = activityStartRef;
    if (start) {
      const s0 = start.getTime();
      for (const e of activityEvents) {
        const el = (new Date(e.timestamp).getTime() - s0) / 1000;
        if (el > m) m = el;
      }
    }
    return Math.max(1, Math.ceil(m));
  });
  // The join code is stable for the page's lifetime; capture it once as the
  // shared-progress key (untrack silences the reactive-read hint).
  const replay = createReplay(untrack(() => data.session.join_code), () => maxReplayT);

  // Mid-replay: engaged with the playhead short of the end. The end-of-session
  // summary blocks (Statistics, LLM costs) are the outcome, so they stay
  // hidden until the replay settles at the finish.
  const replayActive = $derived(replay.engaged && replay.t < replay.total);

  const replayEvents = $derived.by<ActivityEvent[]>(() => {
    if (!replay.engaged || !activityStartRef) return activityEvents;
    const s0 = activityStartRef.getTime();
    const cut = replay.t;
    return activityEvents.filter(
      (e) => (new Date(e.timestamp).getTime() - s0) / 1000 <= cut + 0.001,
    );
  });

  // Own players drive the leaderboard's player-detail links. Live sessions get
  // them from the WS user_players_snapshot; finished sessions (no WS) recover
  // them from the report — only players owned by the logged-in user, since the
  // player detail API authorizes exactly that.
  const effectiveUserPlayers = $derived<import("$lib/types/arena").PlayerSummary[]>(
    userPlayers.length > 0
      ? userPlayers
      : (sessionReport?.players ?? [])
          .filter((p) => p.user_id !== null && p.user_id === data.user?.id)
          .map((p) => ({
            player_id: p.player_id,
            user_id: p.user_id,
            display_name: p.display_name,
            fingerprint: null,
            joined_at: "",
            reconnected_at: null,
            revoked_at: null,
          })),
  );

  const reportMembers = $derived<MemberInfo[]>(
    (sessionReport?.leaderboard ?? []).map((e) => ({
      user_id: e.player_id,
      display_name: e.display_name,
      avatar_url: reportPlayerById.get(e.player_id)?.avatar_url ?? null,
      fingerprint: null,
      username: reportPlayerById.get(e.player_id)?.username ?? null,
      joined_at: "",
    })),
  );

  // Leaderboard rows for the finished-phase UI, augmented with display fields.
  const reportLeaderboard = $derived(
    (sessionReport?.leaderboard ?? []).map((e) => ({
      player_id: e.player_id,
      display_name: e.display_name,
      agent_display_name: e.agent_display_name ?? null,
      total_points: e.total_points,
      tests_passed: e.tests_passed,
      total_wall_ms: e.total_wall_ms,
      avatar_url: reportPlayerById.get(e.player_id)?.avatar_url ?? null,
      fingerprint: null as string | null,
      username: reportPlayerById.get(e.player_id)?.username ?? null,
    })),
  );

  // Finished-session summary line: name the winner instead of a flat
  // "Session complete." — a run deserves a verdict, not just numbers
  // (audit UI-M5). If the viewer played, tell them where they placed.
  const resultSummary = $derived.by<string | null>(() => {
    const rows = [...reportLeaderboard].sort((a, b) => b.total_points - a.total_points);
    if (rows.length === 0) return null;
    const winner = rows[0];
    if (rows.length === 1) {
      return `${winner.display_name} finished with ${winner.total_points} pts.`;
    }
    const ownIds = new Set(effectiveUserPlayers.map((p) => p.player_id));
    let ownPlace: number | null = null;
    rows.forEach((r, i) => {
      if (ownIds.has(r.player_id) && (ownPlace === null || i + 1 < ownPlace)) ownPlace = i + 1;
    });
    const win = `${winner.display_name} wins with ${winner.total_points} pts.`;
    if (ownPlace === 1) return `You win with ${winner.total_points} pts! 🏆`;
    if (ownPlace !== null) return `${win} You placed ${ordinal(ownPlace)} of ${rows.length}.`;
    return win;
  });

  function ordinal(n: number): string {
    const s = ["th", "st", "nd", "rd"];
    const v = n % 100;
    return `${n}${s[(v - 20) % 10] ?? s[v] ?? s[0]}`;
  }

  // ── Lifecycle ─────────────────────────────────────────────────────────────

  onMount(async () => {
    await initAuth();
    const token = getToken();
    const client = new WsSessionClient(data.session.join_code, token);
    wsClient = client;
    client.connect();
  });

  onDestroy(() => {
    if (copyTimeout) clearTimeout(copyTimeout);
    if (celebrationTimer) clearTimeout(celebrationTimer);
    wsClient?.disconnect();
  });
</script>

<svelte:head>
  <title>{pageTitle}</title>
</svelte:head>

<!--
  Public session spectator page at /s/[code]/
  Full-width dark blue countdown bar at top.
  Single-column layout below: header card, project description, phase content.
  Active phase: chart (flex-grow) + leaderboard (fixed 260px) side by side.
 -->
<div class="-mx-6 -mt-8 min-h-screen" style="background: #f4f8fe;">

  {#if showSessionEndBanner}
    <div
      class="flex items-center justify-between gap-3 px-6 py-3 text-sm font-medium
             {endedCancelled ? 'bg-amber-100 text-amber-900' : 'bg-green-100 text-green-900'}"
      role="status"
      data-testid="session-end-banner"
    >
      <span>
        {endedCancelled ? cancelDetail : 'Session finished — final standings below.'}
      </span>
      <button
        type="button"
        class="shrink-0 rounded px-2 py-0.5 text-xs font-semibold hover:bg-black/10"
        onclick={() => (showSessionEndBanner = false)}
      >
        Dismiss
      </button>
    </div>
  {/if}

  <!-- Congratulations popup: appears only when the finish happened while the
       page was open AND every judge has delivered its verdict, so the
       standings it names are final. -->
  <SessionCongratsModal
    open={celebration === 'shown'}
    onClose={() => (celebration = 'dismissed')}
    entries={celebrationEntries}
  />

  <!-- ── TOP BAR: countdown / status (hidden for finished sessions) ───────── -->
  {#if phase !== "finished"}
  <TopBar
    {phase}
    {participants}
    {canControl}
    {controlBusy}
    onControl={(next) => void setSessionStatus(next)}
    onCancel={onCancelClick}
    projectName={data.session.project_name}
  >
    <div class="flex items-center gap-[12px]">
      {#each timerDigits as digit, i (i)}
        <div
          class="flex h-[38px] w-[38px] items-center justify-center rounded-[6px] text-[18px] font-bold text-white"
          style="background: rgba(255,255,255,0.1);"
        >{digit}</div>
        {#if i < 2}
          <span class="text-[16px] font-bold text-white" style="opacity: 0.5;">:</span>
        {/if}
      {/each}
    </div>
  </TopBar>
  {/if}

  <!-- ── MAIN CONTENT ───────────────────────────────────────────────────────── -->
  <!-- Extra bottom room on finished sessions so the fixed replay bar never
       covers the last of the page. -->
  <!-- Finished sessions reserve room for the fixed replay bar, which wraps
       to two rows on narrow screens and needs the extra clearance there. -->
  <div class="mx-auto max-w-[1206px] px-[18px] py-[32px] {phase === 'finished' ? 'pb-[160px] sm:pb-[104px]' : ''}">

    <!-- ── PHASE CONTENT ──────────────────────────────────────────────────── -->

    {#if phase === "lobby"}
      <!-- Lobby: minimal header so join code and status are always visible -->
      <SessionHeaderCard
        joinCode={data.session.join_code}
        projectName={data.session.project_name}
        projectSlug={data.session.project_slug}
        projectId={data.session.project_id}
        badge="lobby"
        showCopy={true}
        {copied}
        onCopy={copyCommand}
        createdAt={data.session.created_at ?? null}
      />
      {#if data.campaign}
        <SessionCampaignCard campaign={data.campaign} />
      {/if}
      {#if wsClient?.protocolMismatch}
        <div class="mb-[12px] rounded-[8px] bg-white px-[20px] py-[14px] text-[13px]" style="color: #fb341c;">
          Update required. Please refresh the page.
        </div>
      {/if}
      {#if wsClient?.degraded}
        <div class="mb-[12px] rounded-[8px] bg-white px-[20px] py-[14px] text-[13px]" style="color: #8fb4ec;">
          {wsClient.degraded}
        </div>
      {/if}
      <SessionLobby {participants} />

    {:else if phase === "active" || phase === "paused"}
      <!-- Chart + Leaderboard side by side -->
      <div class="mb-[20px] flex flex-col gap-[16px] lg:flex-row">

        <!-- Score over time chart (flex-grow) -->
        <div class="min-w-0 flex-grow overflow-hidden rounded-[8px] bg-white px-[24px] py-[20px]">
          <h2 class="mb-[16px] font-heading text-[16px] font-bold" style="color: #363636;">Score over time</h2>
          <ScoreChart {phase} {leaderboard} {reportMembers} {sessionReport} scoreHistory={effectiveScoreHistory} />
        </div>

        <!-- Leaderboard (fixed 300px on large screens) -->
        <div class="w-full overflow-hidden rounded-[8px] bg-white px-[16px] py-[20px] lg:w-[300px] lg:shrink-0 lg:self-start">
          <h2 class="mb-[16px] px-[8px] font-heading text-[16px] font-bold" style="color: #363636;">Leaderboard</h2>
          <LeaderboardList
            entries={mergedLeaderboardWithInfo}
            joinCode={data.session.join_code}
            {userPlayers}
            {isAdmin}
            emptyMessage="Waiting for first results…"
          />
        </div>

      </div>

      <!-- Header card: join code, status badge, project name, copy command, collapsible description -->
      <SessionHeaderCard
        joinCode={data.session.join_code}
        projectName={data.session.project_name}
        projectSlug={data.session.project_slug}
        projectId={data.session.project_id}
        badge={phase === "active" ? "live" : "paused"}
        showCopy={true}
        {copied}
        onCopy={copyCommand}
        description={data.session.project_description}
        startedAt={data.session.started_at ?? null}
        finishedAt={data.session.finished_at ?? null}
        createdAt={data.session.created_at ?? null}
      />

      {#if data.campaign}
        <SessionCampaignCard campaign={data.campaign} />
      {/if}

      <!-- Activity log (Statistics is a finished-session summary — omitted
           while the session is still running) -->
      <SessionCostsPanel sessionId={data.session.id} isAdmin={data.isAdmin} />
      <ActivityLog
        events={activityEvents}
        startedAt={activityStartRef}
        {avatarByPlayerId}
        sessionId={data.session.id}
      />

    {:else}
      <!-- Finished: the page is a report, and most visitors arrive on a shared
           link — it names the session before it charts it. A running session
           keeps the board on top instead: there the moving scoreboard is the
           reason the page is open. -->
      <!-- Header card: join code, Complete badge, project link — no join command, collapsible description -->
      <SessionHeaderCard
        joinCode={data.session.join_code}
        projectName={data.session.project_name}
        projectSlug={data.session.project_slug}
        projectId={data.session.project_id}
        badge={judgingPending ? "judging" : "complete"}
        showCopy={false}
        showShare={true}
        {copied}
        onCopy={copyCommand}
        description={data.session.project_description}
        startedAt={data.session.started_at ?? null}
        finishedAt={effectiveFinishedAt}
        createdAt={data.session.created_at ?? null}
      />


      <!-- Replay controls: sweep a playhead through the session; the chart
           draws in and the activity feed reveals events up to it. Admin-only,
           and only where the instance offers it (Settings → General). -->
      {#if data.isAdmin && data.replayEnabled !== false && effectiveScoreHistory.length > 1}
        <SessionReplay
          total={replay.total}
          t={replay.t}
          playing={replay.playing}
          speed={replay.speed}
          onSeek={replay.seek}
          onToggle={replay.toggle}
          onSpeed={replay.setSpeed}
        />
      {/if}

      <!-- Chart + Final leaderboard -->
      <div class="mb-[20px] flex flex-col gap-[16px] lg:flex-row">

        <!-- Score over time chart -->
        <div class="min-w-0 flex-grow overflow-hidden rounded-[8px] bg-white px-[24px] py-[20px]">
          <h2 class="mb-[16px] font-heading text-[16px] font-bold" style="color: #363636;">Score over time</h2>
          <ScoreChart {phase} {leaderboard} {reportMembers} {sessionReport} scoreHistory={effectiveScoreHistory} revealUntil={replay.revealUntil} />
        </div>

        <!-- Final leaderboard -->
        <div class="w-full overflow-hidden rounded-[8px] bg-white px-[16px] py-[20px] lg:w-[300px] lg:shrink-0 lg:self-start">
          <h2 class="mb-[4px] px-[8px] font-heading text-[16px] font-bold" style="color: #363636;">Final Results</h2>
          <p class="mb-[16px] px-[8px] text-sm" style="color: #8fb4ec;" data-testid="result-summary">
            {resultSummary ?? "Session complete."}
          </p>
          <LeaderboardList
            entries={reportLeaderboard}
            joinCode={data.session.join_code}
            userPlayers={effectiveUserPlayers}
            {isAdmin}
            detailLabel="Report"
            emptyMessage="No results recorded."
          />
        </div>

      </div>

      {#if data.campaign}
        <SessionCampaignCard campaign={data.campaign} />
      {/if}

      <!-- Statistics + Activity log. The outcome blocks are the session's
           result, so a replay in flight hides them until it reaches the
           finish; the activity feed below replays throughout. -->
      {#if !replayActive}
        <SessionPlayerStatsBlock stats={playerStats} />
        <SessionCostsPanel sessionId={data.session.id} isAdmin={data.isAdmin} />
      {/if}
      <ActivityLog
        events={replayEvents}
        startedAt={activityStartRef}
        {avatarByPlayerId}
        sessionId={data.session.id}
      />

    {/if}

  </div>
</div>

<style>
  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.3; }
  }
</style>
