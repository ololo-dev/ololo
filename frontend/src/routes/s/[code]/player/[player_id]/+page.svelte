<script lang="ts">
  import { browser } from '$app/environment'
  import { page } from '$app/state'
  import { untrack } from 'svelte'
  import { sessionLinkLabel } from '$lib/session-status'
  import type { PageData } from './$types'
  import type {
    PlayerJudgeScoredPayload,
    PlayerJudgeStatusPayload,
    PlayerProbeEntry,
    PlayerTaskSummaryEntry,
    PlayerHistoryCommit,
    PlayerHistoryResponse,
    PlayerMemoryEntry,
    PlayerMemoryResponse,
    TaskStatsEntry,
    TaskStatsResponse,
  } from '$lib/types/arena'
  import { WsPlayerClient } from '$lib/ws-player.svelte'
  import { WsSessionClient } from '$lib/ws-session.svelte'
  import PlayerHistory from '$lib/components/PlayerHistory.svelte'
  import { attributeCommits } from '$lib/sessions/attribution'
  import { resolveBaseline } from '$lib/sessions/baseline'
  import PlayerHeader from '$lib/components/sessions/PlayerHeader.svelte'
  import PlayerArtifactBanner from '$lib/components/sessions/PlayerArtifactBanner.svelte'
  import PlayerTaskList from '$lib/components/sessions/PlayerTaskList.svelte'
  import PlayerTaskChat from '$lib/components/sessions/PlayerTaskChat.svelte'
  import SessionReportPanel from '$lib/components/sessions/SessionReportPanel.svelte'
  import PlayerSectionTabs from '$lib/components/sessions/PlayerSectionTabs.svelte'
  import PlayerJudgesTab from '$lib/components/sessions/PlayerJudgesTab.svelte'
  import PlayerStatsTab from '$lib/components/sessions/PlayerStatsTab.svelte'
  import PlayerMemoryTab from '$lib/components/sessions/PlayerMemoryTab.svelte'
  import type { SectionTabId } from '$lib/sessions/player-tabs'
  import { getPlayerEventLog } from '$lib/api/sessions'
  import { getSessionReport } from '$lib/api'
  import type { SessionReportResponse } from '$lib/api/types'
  import SessionReplay from '$lib/components/session/SessionReplay.svelte'
  import SessionCongratsModal, {
    type CongratsSummary,
  } from '$lib/components/session/SessionCongratsModal.svelte'
  import { createReplay } from '$lib/replay.svelte'

  const { data }: { data: PageData } = $props()

  const sessionCode = $derived(page.params.code ?? '')

  // WS client — declared early so deriveds can read its reactive snapshot.
  let wsClient = $state<WsPlayerClient | null>(null)
  // ponytail: separate session-channel client — it carries the server-authoritative
  // running_countdown frame (same value ololo's TUI renders). The player channel
  // does not forward countdown frames.
  let sessionClient = $state<WsSessionClient | null>(null)

  // Live data: prefer WS snapshot when connected, fall back to SSR data.
  // Reading wsClient.snapshot directly in $derived makes reactivity automatic.
  const liveSnapshot = $derived(
    wsClient?.snapshot ?? data.snapshot,
  )

  // ── Replay ────────────────────────────────────────────────────────────
  // A finished session can be re-watched: a playhead sweeps session time and
  // every collection below is filtered to what had happened by then, so the
  // Chat and Details views (probes, commits, judge verdicts, tasks) reveal
  // progressively. Off (cutoff null) until the user engages the control bar.
  function ts(s: string | null | undefined): number | null {
    if (!s) return null
    const t = Date.parse(s)
    return Number.isNaN(t) ? null : t
  }
  // A probe is revealed when it resolves (or, lacking that, when dispatched).
  function probeRevealMs(p: PlayerProbeEntry): number | null {
    return ts(p.resolved_at) ?? ts(p.dispatched_at) ?? ts(p.updated_at)
  }

  const allProbes = $derived<PlayerProbeEntry[]>(liveSnapshot.probes ?? [])
  const allTasks = $derived<PlayerTaskSummaryEntry[]>(liveSnapshot.tasks)

  // task_id → the moment its result is decided (its last probe resolving),
  // so a passed/failed badge only appears once the playhead reaches it.
  const taskResultMs = $derived.by(() => {
    const m = new Map<string, number>()
    for (const p of allProbes) {
      const r = probeRevealMs(p)
      if (r == null) continue
      const prev = m.get(p.task_id)
      if (prev == null || r > prev) m.set(p.task_id, r)
    }
    return m
  })

  // Timeline bounds across every timestamped event, anchored at session start.
  const replayBounds = $derived.by(() => {
    let min = Infinity
    let max = -Infinity
    const consider = (ms: number | null) => {
      if (ms == null) return
      if (ms < min) min = ms
      if (ms > max) max = ms
    }
    for (const p of allProbes) {
      consider(probeRevealMs(p))
      consider(ts(p.dispatched_at))
    }
    for (const c of historyCommits) consider(ts(c.author_time))
    for (const j of allJudgeResults) consider(ts(j.created_at))
    const started = ts(liveSnapshot.session_started_at)
    // Anchor the origin at session start so the playhead means the same thing
    // here as on the dashboard — the two share one progress value by code.
    const t0 = started != null ? Math.min(started, min) : min
    if (!Number.isFinite(t0) || !Number.isFinite(max) || max <= t0) return null
    return { t0, totalSecs: Math.max(1, Math.ceil((max - t0) / 1000)) }
  })
  const replayTotal = $derived(replayBounds?.totalSecs ?? 0)
  // sessionCode is stable for the page's lifetime; capture it once as the
  // shared-progress key (untrack silences the reactive-read hint).
  const replay = createReplay(untrack(() => sessionCode), () => replayTotal)
  // A single replay cutoff for the whole page: while a replay is engaged both
  // the Chat and the Details views reveal everything — tasks, probes, changes,
  // verdicts, evaluations — up to the playhead, so Details replays the project
  // exactly as Chat does.
  const replayCutoffMs = $derived(
    replay.engaged && replayBounds ? replayBounds.t0 + replay.t * 1000 : null,
  )
  // Admin-only, and only where the instance offers it: regular users never
  // see the control bar, so the cutoff stays disengaged and the page shows
  // full data. The switch lives in Settings → General.
  const replayAvailable = $derived.by(
    () =>
      data.isAdmin &&
      data.replayEnabled !== false &&
      sessionFinished &&
      replayBounds != null &&
      replayTotal > 2,
  )
  // Mid-replay: engaged and the playhead hasn't reached the end yet. Consumers
  // hide end-of-session content (the summary card) until it settles here.
  const replayActive = $derived(replay.engaged && replay.t < replay.total)

  const probes = $derived.by<PlayerProbeEntry[]>(() => {
    const cut = replayCutoffMs
    if (cut == null) return allProbes
    return allProbes.filter((p) => {
      const r = probeRevealMs(p)
      return r != null && r <= cut
    })
  })
  const tasks = $derived.by<PlayerTaskSummaryEntry[]>(() => {
    const cut = replayCutoffMs
    if (cut == null) return allTasks
    return allTasks.map((t) => {
      const done = taskResultMs.get(t.task_id)
      return done != null && done <= cut
        ? t
        : { ...t, result: null, scheduler_state: null }
    })
  })
  const totalTasks = $derived(liveSnapshot.total_tasks)
  const score = $derived(liveSnapshot.score)
  const rank = $derived(liveSnapshot.rank)
  const agentDisplayName = $derived(liveSnapshot.agent_display_name ?? null)
  const completionStatus = $derived(liveSnapshot.completion_status ?? null)
  const avatarUrl = $derived(liveSnapshot.avatar_url ?? null)
  const nextProbeAt = $derived<string | null>(
    wsClient?.snapshot?.next_probe_at ?? data.snapshot.next_probe_at,
  )
  const sessionFinished = $derived(
    wsClient?.sessionFinished ?? !data.live,
  )
  const agentConnected = $derived<boolean | null>(
    wsClient?.agentConnected ?? data.snapshot.agent_connected ?? null,
  )
  // Session-end notification: the page was opened on a live session and the
  // end arrived while watching — worth an explicit banner, not just a badge
  // flip in the corner.
  let sessionEndSeen = $state(false)
  let showSessionEndBanner = $state(false)
  let judgingWasInProgress = $state(false)
  $effect(() => {
    if (sessionFinished && !sessionEndSeen) {
      sessionEndSeen = true
      if (data.live) showSessionEndBanner = true
    }
    // Remember that the judges-working dialog was showing, so that when it
    // clears we surface the plain end banner even on a page opened after the
    // session already ended (where data.live was false).
    if (judgingInProgress) judgingWasInProgress = true
    else if (judgingWasInProgress && sessionFinished) {
      judgingWasInProgress = false
      showSessionEndBanner = true
    }
  })
  const sessionEndedAsCancelled = $derived(
    (wsClient?.snapshot?.session_status ?? data.snapshot.session_status) === 'cancelled',
  )

  // ── End-of-session celebration ──────────────────────────────────────────
  // Mirrors the dashboard: a finish that arrives live earns a popup, but only
  // after the session-channel `session_settled` frame — the server's word
  // that every owed judge run landed and the standings are final. One report
  // fetch then supplies the final leaderboard plus this player's summary.
  let celebration = $state<'idle' | 'waiting' | 'shown' | 'dismissed'>('idle')
  let celebrationTimer: ReturnType<typeof setTimeout> | null = null
  let celebrationReport = $state<SessionReportResponse | null>(null)

  function finalizeCelebration() {
    if (celebration !== 'waiting') return
    // Settled means the last judge has spoken, and the report is written after
    // all of them. It arrives in the snapshot rather than as a verdict frame —
    // it scores nothing — so this is what puts it on the page without a reload.
    wsClient?.refreshSnapshot()
    if (celebrationTimer) {
      clearTimeout(celebrationTimer)
      celebrationTimer = null
    }
    if (!data.sessionId) {
      celebration = 'shown' // fall back to the live WS leaderboard
      return
    }
    getSessionReport(data.sessionId)
      .then((r) => (celebrationReport = r))
      .finally(() => (celebration = 'shown'))
  }

  $effect(() => {
    if (
      celebration === 'idle' &&
      data.live &&
      sessionFinished &&
      !sessionEndedAsCancelled
    ) {
      celebration = 'waiting'
      // One-shot safety net for a dropped WS; not a polling loop.
      celebrationTimer = setTimeout(() => finalizeCelebration(), 10 * 60 * 1000)
    }
    if (celebration === 'waiting' && sessionClient?.settled) finalizeCelebration()
    return () => {
      if (celebrationTimer) clearTimeout(celebrationTimer)
    }
  })

  const celebrationSummary = $derived.by<CongratsSummary | null>(() => {
    const report = celebrationReport
    if (!report) return null
    const standings = [...report.leaderboard].sort((a, b) => b.total_points - a.total_points)
    const myId = data.snapshot.player_id
    const idx = standings.findIndex((e) => e.player_id === myId)
    if (idx < 0) return null
    const me = standings[idx]
    return {
      place: idx + 1,
      of: standings.length,
      points: me.total_points,
      testsPassed: me.tests_passed,
    }
  })
  // Judges still to settle for THIS player once the session ended. Names are
  // deduped so the dialog lists each judge once, not once per task.
  const pendingJudges = $derived.by(() => {
    const seen = new Set<string>()
    const out: string[] = []
    for (const st of judgeStatuses) {
      if (st.status === 'running' || st.status === 'pending') {
        if (!seen.has(st.judge_name)) {
          seen.add(st.judge_name)
          out.push(st.judge_name)
        }
      }
    }
    return out
  })
  // Judges that never settle (a run lost to a deploy, a session outside the
  // recovery window) must not keep the "judges scoring" dialog up forever:
  // past this long after the last sign of life the page shows the plain end
  // banner instead. Server-side settling gives up after 10 minutes; 30 is
  // generous.
  const JUDGES_STALE_AFTER_MS = 30 * 60 * 1000
  const judgingStale = $derived.by(() => {
    let lastSignal = sessionEndsAt ? Date.parse(sessionEndsAt) : 0
    for (const st of judgeStatuses) {
      if (st.updated_at) {
        const t = Date.parse(st.updated_at)
        if (t > lastSignal) lastSignal = t
      }
    }
    return lastSignal > 0 && nowMs - lastSignal > JUDGES_STALE_AFTER_MS
  })
  // The session has ended and this player's judges have not all settled: show
  // the "judges still working" dialog instead of the plain end banner.
  const judgingInProgress = $derived(
    sessionFinished && !sessionEndedAsCancelled && pendingJudges.length > 0 && !judgingStale,
  )

  const cancelDetail = $derived.by(() => {
    const reason = wsClient?.cancelReason
    const by = wsClient?.cancelledBy
    if (reason === 'idle_timeout')
      return 'Session ended automatically — no players were connected.'
    if (reason === 'user')
      return by ? `Session was cancelled by ${by}.` : 'Session was cancelled.'
    return 'Session was cancelled — no more probes will run.'
  })
  const sessionPaused = $derived(
    wsClient?.sessionPaused ??
      data.snapshot.session_status === 'paused',
  )
  const sessionEndsAt = $derived<string | null>(
    wsClient?.snapshot?.session_ends_at ?? data.snapshot.session_ends_at ?? null,
  )
  let nowMs = $state(Date.now())

  // History state — seeded from SSR load, refreshable client-side.
  let historyCommits = $state<PlayerHistoryCommit[]>(
    untrack(() => data.history?.commits ?? []),
  )
  let historyLoading = $state(false)
  let historyError = $state<string | null>(null)

  async function refreshHistory(): Promise<void> {
    if (!browser) return
    historyLoading = true
    historyError = null
    try {
      const resp = await fetch(
        `/api/sessions/${encodeURIComponent(sessionCode)}/players/${encodeURIComponent(data.snapshot.player_id)}/history`,
      )
      if (!resp.ok) {
        historyError = `Failed to load history (HTTP ${resp.status})`
        return
      }
      const body = (await resp.json()) as PlayerHistoryResponse
      historyCommits = body.commits
    } catch (e) {
      historyError = `Failed to load history: ${e instanceof Error ? e.message : String(e)}`
    } finally {
      historyLoading = false
    }
  }

  // Task agent statistics — seeded from SSR load, refreshed when tasks pass
  // (ololo reports stats right after task completion).
  let taskStatsEntries = $state<TaskStatsEntry[]>(
    untrack(() => data.taskStats?.entries ?? []),
  )

  async function refreshTaskStats(): Promise<void> {
    if (!browser) return
    try {
      const resp = await fetch(
        `/api/sessions/${encodeURIComponent(sessionCode)}/players/${encodeURIComponent(data.snapshot.player_id)}/task-stats`,
      )
      if (resp.ok) {
        const body = (await resp.json()) as TaskStatsResponse
        taskStatsEntries = body.entries
      }
    } catch {
      // non-fatal
    }
  }

  // Session memory — fetched client-side; the tab only appears when the
  // project declares a memory schema (memoryEnabled).
  let memoryEnabled = $state(false)
  let memoryEntries = $state<PlayerMemoryEntry[]>([])
  let memoryUpdatedAt = $state<string | null>(null)
  let memoryLoading = $state(false)
  let memoryError = $state<string | null>(null)

  async function refreshMemory(): Promise<void> {
    if (!browser) return
    memoryLoading = true
    memoryError = null
    try {
      const resp = await fetch(
        `/api/sessions/${encodeURIComponent(sessionCode)}/players/${encodeURIComponent(data.snapshot.player_id)}/memory`,
      )
      if (!resp.ok) {
        // Forbidden/unauthed viewers simply don't get the tab.
        memoryEnabled = false
        return
      }
      const body = (await resp.json()) as PlayerMemoryResponse
      memoryEnabled = body.enabled
      memoryEntries = body.entries
      memoryUpdatedAt = body.updated_at
    } catch (e) {
      memoryError = `Failed to load memory: ${e instanceof Error ? e.message : String(e)}`
    } finally {
      memoryLoading = false
    }
  }

  $effect(() => {
    if (!browser) return
    void refreshMemory()
  })

  // Stats grouped by task ordinal (entries always carry the ordinal; the
  // task id may be null if the project task was deleted).
  const taskStatsByOrdinal = $derived.by(() => {
    const map = new Map<number, TaskStatsEntry>()
    for (const e of taskStatsEntries) map.set(e.task_ordinal, e)
    return map
  })

  const currentOrdinal = $derived.by(() => {
    // During replay, a task appears as soon as its first probe is revealed.
    if (replayCutoffMs != null) {
      let max = 0
      for (const t of tasks) if (probesByTask.has(t.task_id) && t.ordinal > max) max = t.ordinal
      return max
    }
    const current = tasks.find((t) => t.scheduler_state !== null)
    if (current) return current.ordinal
    // No active task — show only tasks that have been attempted (result or probes).
    const attempted = tasks.filter(
      (t) => t.result !== null || probesByTask.has(t.task_id),
    )
    return attempted.length > 0
      ? Math.max(...attempted.map((t) => t.ordinal))
      : 0
  })

  const taskById = $derived.by(() => {
    const map = new Map<string, PlayerTaskSummaryEntry>()
    for (const t of tasks) map.set(t.task_id, t)
    return map
  })

  // A task is "passed" when the server marks it completed via
  // player_task_delta (result.status === 'completed'). The server
  // advances the scheduler past the task once its adapted tests all
  // pass, so this is the authoritative signal — independent of how
  // many failed probe attempts preceded the pass.
  const PASSED_TASK_STATUSES = new Set(['completed'])

  function isTaskPassed(taskId: string): boolean {
    const task = taskById.get(taskId)
    return (
      !!task?.result?.status && PASSED_TASK_STATUSES.has(task.result.status)
    )
  }

  const passedTasksCount = $derived(
    tasks.filter((t) => isTaskPassed(t.task_id)).length,
  )

  // Commit attribution: parse feat(task_id): messages; detect session-start
  // baseline. Task windows (opened by each task's first probe) catch legacy
  // commits without a task-id prefix so every commit lands on some task.
  const taskWindows = $derived.by(() => {
    const firstProbe = new Map<string, string>()
    for (const p of probes) {
      if (!p.dispatched_at) continue
      const prev = firstProbe.get(p.task_id)
      if (!prev || p.dispatched_at < prev) firstProbe.set(p.task_id, p.dispatched_at)
    }
    return tasks.map((t) => ({
      task_id: t.task_id,
      ordinal: t.ordinal,
      started_at: firstProbe.get(t.task_id) ?? null,
    }))
  })
  // Commits revealed up to the playhead (all of them when not replaying).
  const commits = $derived.by<PlayerHistoryCommit[]>(() => {
    const cut = replayCutoffMs
    if (cut == null) return historyCommits
    return historyCommits.filter((c) => {
      const t = ts(c.author_time)
      return t != null && t <= cut
    })
  })
  const attribution = $derived(attributeCommits(commits, taskWindows))

  // Probes grouped by task_id for per-task panel rendering.
  const probesByTask = $derived.by(() => {
    const map = new Map<string, PlayerProbeEntry[]>()
    for (const p of probes) {
      const arr = map.get(p.task_id)
      if (arr) arr.push(p)
      else map.set(p.task_id, [p])
    }
    return map
  })

  // Judge results — WS client (seeded from its snapshot + live judge_scored
  // frames) with the SSR snapshot as fallback before the WS connects.
  const allJudgeResults = $derived<PlayerJudgeScoredPayload[]>(
    wsClient?.judgeResults?.length
      ? wsClient.judgeResults
      : (data.snapshot.judge_results ?? []),
  )
  // Verdicts revealed up to the playhead (all when not replaying).
  const judgeResults = $derived.by<PlayerJudgeScoredPayload[]>(() => {
    const cut = replayCutoffMs
    if (cut == null) return allJudgeResults
    return allJudgeResults.filter((j) => {
      const t = ts(j.created_at)
      return t != null && t <= cut
    })
  })

  // Judge results grouped by task_id, stable insertion order.
  const judgeResultsByTask = $derived.by(() => {
    const map = new Map<string, PlayerJudgeScoredPayload[]>()
    for (const j of judgeResults) {
      const arr = map.get(j.task_id)
      if (arr) arr.push(j)
      else map.set(j.task_id, [j])
    }
    return map
  })

  // Judge lifecycle (pending/running/scored/failed) — WS client with the
  // SSR snapshot as fallback, mirroring judgeResults above.
  const allJudgeStatuses = $derived<PlayerJudgeStatusPayload[]>(
    wsClient?.judgeStatuses?.length
      ? wsClient.judgeStatuses
      : (data.snapshot.judge_statuses ?? []),
  )
  // Judge lifecycle rows revealed up to the playhead. A row lands when it last
  // changed (updated_at); rows lacking that are dropped only while replaying.
  const judgeStatuses = $derived.by<PlayerJudgeStatusPayload[]>(() => {
    const cut = replayCutoffMs
    if (cut == null) return allJudgeStatuses
    return allJudgeStatuses.filter((s) => {
      const t = ts(s.updated_at)
      return t != null && t <= cut
    })
  })

  const judgeStatusesByTask = $derived.by(() => {
    const map = new Map<string, PlayerJudgeStatusPayload[]>()
    for (const s of judgeStatuses) {
      const arr = map.get(s.task_id)
      if (arr) arr.push(s)
      else map.set(s.task_id, [s])
    }
    return map
  })

  // Probes-by-type pass counter: group by adapted_test_id, count groups with ≥1 pass.
  function taskTypeProgress(taskId: string): { passed: number; total: number } {
    const taskProbes = probesByTask.get(taskId) ?? []
    const byType = new Map<string, boolean>()
    for (const p of taskProbes) {
      const hasPass =
        p.outcome === 'pass' ||
        p.result?.status === 'passed' ||
        p.result?.status === 'correct'
      const prev = byType.get(p.adapted_test_id)
      byType.set(p.adapted_test_id, (prev ?? false) || hasPass)
    }
    let passed = 0
    for (const v of byType.values()) if (v) passed++
    return { passed, total: byType.size }
  }

  // Tasks filtered to ordinal <= current_ordinal (hide future tasks).
  const visibleTasks = $derived(
    [...tasks]
      .filter((t) => t.ordinal <= currentOrdinal)
      .sort((a, b) => a.ordinal - b.ordinal),
  )

  type TaskSortKey = 'date' | 'status'
  type TaskSort = { key: TaskSortKey; dir: 'asc' | 'desc' }
  let taskSort = $state<TaskSort>({ key: 'date', dir: 'desc' })

  function cycleSort(key: TaskSortKey): void {
    if (taskSort.key !== key) {
      taskSort = { key, dir: key === 'date' ? 'desc' : 'asc' }
    } else if (taskSort.dir === 'desc') {
      taskSort = { key, dir: 'asc' }
    } else {
      taskSort = { key, dir: 'desc' }
    }
  }

  const STATUS_ORDER: Record<string, number> = {
    completed: 0,
    passed: 0,
    correct: 0,
    failed: 1,
    error: 1,
    incorrect: 1,
    pending: 2,
  }

  function taskLatestDate(taskId: string): number {
    const ps = probesByTask.get(taskId) ?? []
    let max = 0
    for (const p of ps) {
      if (p.dispatched_at) {
        const t = new Date(p.dispatched_at).getTime()
        if (t > max) max = t
      }
    }
    return max
  }

  const sortedTasks = $derived.by(() => {
    const arr = [...visibleTasks]
    if (taskSort.key === 'date') {
      arr.sort((a, b) => taskLatestDate(b.task_id) - taskLatestDate(a.task_id))
      if (taskSort.dir === 'asc') arr.reverse()
    } else {
      arr.sort((a, b) => {
        const sa = STATUS_ORDER[a.result?.status ?? 'pending'] ?? 2
        const sb = STATUS_ORDER[b.result?.status ?? 'pending'] ?? 2
        if (sa !== sb) return sa - sb
        return taskLatestDate(b.task_id) - taskLatestDate(a.task_id)
      })
      if (taskSort.dir === 'desc') arr.reverse()
    }
    return arr
  })

  // Per-task changes: baseline-resolved commit sets.
  const changesByTask = $derived.by(() => {
    const map = new Map<
      string,
      { commits: PlayerHistoryCommit[]; mode: 'range' | 'per-commit' }
    >()
    for (const task of visibleTasks) {
      const r = resolveBaseline(
        task.ordinal,
        tasks,
        attribution.attributed,
        attribution.baseline,
      )
      map.set(task.task_id, { commits: r.commits, mode: r.mode })
    }
    return map
  })

  // ── Admin: download the player's full event log (JSONL) ─────────────────
  let eventLogBusy = $state(false)
  let eventLogError = $state('')
  async function downloadEventLog() {
    if (!sessionCode || !data.snapshot?.player_id) return
    eventLogBusy = true
    eventLogError = ''
    try {
      const raw = await getPlayerEventLog(sessionCode, data.snapshot.player_id)
      const blob = new Blob([raw], { type: 'application/x-ndjson' })
      const url = URL.createObjectURL(blob)
      const a = document.createElement('a')
      a.href = url
      a.download = `session-${sessionCode}-player-${data.snapshot.player_id}-events.jsonl`
      a.click()
      URL.revokeObjectURL(url)
    } catch {
      eventLogError = 'No event log recorded for this player yet.'
    } finally {
      eventLogBusy = false
    }
  }

  // ── Page-level section tabs ─────────────────────────────────────────────
  let section = $state<SectionTabId>('tasks')

  // The chat transcript vs the full record. A player mid-race follows the
  // conversation; someone reviewing a finished session wants everything.
  // The default follows the session's state. An explicit choice sticks.
  type TaskView = 'report' | 'chat' | 'details'
  // Per session: a choice is about the session you are looking at, not a
  // preference for life. A global key meant one old click on "Details" would
  // hide every future report behind a tab.
  // The player is fixed for this page's lifetime; capture the id once,
  // the same way the replay does with the session code.
  const VIEW_KEY = `player:task-view:${untrack(() => data.snapshot.player_id)}`
  const VALID_VIEWS: readonly string[] = ['report', 'chat', 'details']
  function storedView(): TaskView | null {
    // Older builds also stored 'slides' — treat anything unknown as unset.
    const v = localStorage.getItem(VIEW_KEY)
    return v !== null && VALID_VIEWS.includes(v) ? (v as TaskView) : null
  }
  let viewChoice = $state<TaskView | null>(browser ? storedView() : null)
  // A finished session opens on the report — it is the answer to "how did I
  // do", and it summarises everything the other views show in detail. Live
  // sessions still open on the chat, and a finished session with no report
  // falls back to the full record.
  const sessionReport = $derived(
    wsClient?.snapshot?.session_report ?? data.snapshot.session_report ?? null,
  )
  // Details is the inspection view — probe commands, raw output — and it
  // follows the same boundary the server already enforces on the history
  // endpoint: the run's owner and admins see it, a spectator does not.
  const canInspect = $derived(!data.inspectRestricted)
  const defaultView = $derived<TaskView>(
    data.live
      ? 'chat'
      : sessionReport || data.judgesSettling
        ? 'report'
        : canInspect
          ? 'details'
          : 'chat',
  )
  const taskView = $derived.by<TaskView>(() => {
    const chosen = viewChoice ?? defaultView
    // A remembered choice from someone's own run must not open another
    // player's inspection view.
    return chosen === 'details' && !canInspect ? defaultView : chosen
  })

  // The report only makes sense once the session is over; while it runs the
  // toggle stays the two views it always was.
  const viewOptions = $derived(
    [
      ...(data.live ? [] : [{ id: 'report' as const, label: 'Report' }]),
      { id: 'chat' as const, label: 'Chat' },
      ...(canInspect ? [{ id: 'details' as const, label: 'Details' }] : []),
    ],
  )

  function setTaskView(v: TaskView) {
    viewChoice = v
    if (browser) localStorage.setItem(VIEW_KEY, v)
  }

  // sha → provenance label, so the flat Changes list still shows what each
  // commit belongs to (the per-task grouping lives in the Tasks tab).
  const commitLabels = $derived.by(() => {
    const map = new Map<string, string>()
    if (attribution.baseline) {
      map.set(attribution.baseline.sha, 'Session start')
    }
    for (const [taskId, commits] of attribution.attributed) {
      const task = taskById.get(taskId)
      if (!task) continue
      for (const c of commits) map.set(c.sha, `Task #${task.ordinal}`)
    }
    return map
  })

  // Judge count for the tab badge: lifecycle rows are authoritative, but
  // sessions predating them only have verdicts.
  const judgeCount = $derived(
    Math.max(judgeStatuses.length, judgeResults.length),
  )

  // Open-ended evaluation state (criteria, TODO, artifacts) from the
  // snapshot; the tab appears only for open-ended projects.
  const allEvaluations = $derived(
    (wsClient?.snapshot?.evaluations?.length
      ? wsClient.snapshot.evaluations
      : data.snapshot.evaluations) ?? [],
  )
  // While replaying, a criterion's per-judge scores appear when that judge's
  // verdict lands, and measurements appear at their timestamp.
  const evaluations = $derived.by(() => {
    const cut = replayCutoffMs
    if (cut == null) return allEvaluations
    const judgeAt = new Map<string, number>() // `${task_id}|${judge_slug}` → verdict time
    for (const j of allJudgeResults) {
      const t = ts(j.created_at)
      if (t != null) judgeAt.set(`${j.task_id}|${j.judge_slug}`, t)
    }
    return allEvaluations.map((ev) => ({
      ...ev,
      criteria: ev.criteria.map((c) => ({
        ...c,
        scores: (c.scores ?? []).filter((s) => {
          const t = judgeAt.get(`${ev.task_id}|${s.judge_slug}`)
          return t != null && t <= cut
        }),
      })),
      measurements: (ev.measurements ?? []).filter((m) => {
        const t = ts(m.at)
        return t != null && t <= cut
      }),
    }))
  })
  const evaluationsByTask = $derived(new Map(evaluations.map((ev) => [ev.task_id, ev])))

  // Interactive artifact requests still waiting on the participant — the
  // amber banner with the per-request countdown. artifact_awaited frames
  // trigger a snapshot refresh in the WS client, so this stays live.
  const pendingArtifacts = $derived(
    evaluations
      .flatMap((ev) => ev.pending_artifacts ?? [])
      .filter((r) => new Date(r.deadline_at).getTime() > nowMs),
  )

  // Captures land mid-run as artifact_received events on the session channel
  // (the player channel never carries them) — refresh the player snapshot so
  // the screenshot gallery updates the moment a capture arrives, not only
  // after the verdict.
  const artifactEventsSeen = $derived(
    sessionClient?.activityLog.filter(
      (e) => e.kind === 'artifact_received' && e.player_id === data.snapshot.player_id,
    ).length ?? 0,
  )
  let lastArtifactSeen = $state(-1)
  $effect(() => {
    const n = artifactEventsSeen
    const prev = lastArtifactSeen
    if (n === prev) return
    lastArtifactSeen = n
    if (prev >= 0 && n > prev) wsClient?.refreshSnapshot()
  })

  // A spectator reads the run — report, judges, stats — but the inspection
  // surface (diffs, files, memory) belongs to the player and admins, so its
  // tabs disappear rather than render broken. (Memory hides itself the same
  // way: its endpoint answers 403 and memoryEnabled stays false.)
  const sectionTabs = $derived([
    { id: 'tasks' as const, label: 'Tasks', count: visibleTasks.length },
    ...(data.inspectRestricted
      ? []
      : [{ id: 'changes' as const, label: 'Changes', count: commits.length }]),
    { id: 'judges' as const, label: 'Judges', count: judgeCount },
    { id: 'stats' as const, label: 'Stats', count: taskStatsEntries.length },
    ...(memoryEnabled
      ? [{ id: 'memory' as const, label: 'Memory', count: memoryEntries.length }]
      : []),
  ])

  // Refetch git history when a task newly passes — ololo pushes a commit
  // to the per-player repo on task completion, so the history changes.
  let lastPassedCount = $state(
    untrack(
      () =>
        data.snapshot.tasks.filter((t) => t.result?.status === 'completed')
          .length,
    ),
  )
  $effect(() => {
    if (!browser) return
    const count = passedTasksCount
    if (count > lastPassedCount) {
      lastPassedCount = count
      refreshHistory()
      // ololo reports task stats shortly after completion, but collection
      // can take a while on large local agent stores — retry with backoff
      // so the Stats tab appears without a page reload.
      for (const delay of [5000, 20000, 60000]) {
        setTimeout(() => void refreshTaskStats(), delay)
      }
      // Memory extraction runs server-side after the task's snapshot commit
      // lands (commit wait + LLM call) — same backoff window applies.
      for (const delay of [5000, 20000, 60000]) {
        setTimeout(() => void refreshMemory(), delay)
      }
    }
  })

  // A judge verdict implies the stats report landed (judge runs are gated on
  // it server-side) — refetch so the Stats tab appears alongside the verdict.
  // It also implies the delivery's snapshot commits (flag, done-note) were
  // pushed — refetch history so the chat shows them without a reload.
  let lastJudgeCount = $state(
    untrack(() => data.snapshot.judge_results?.length ?? 0),
  )
  $effect(() => {
    if (!browser) return
    const count = judgeResults.length
    if (count > lastJudgeCount) {
      lastJudgeCount = count
      void refreshTaskStats()
      void refreshHistory()
    }
  })

  $effect(() => {
    // Open the socket while the session is live OR while its judges are still
    // settling after finish — that post-finish window is exactly when judge
    // verdicts stream in, and closing the socket then is why they used to only
    // appear on refresh. data.token is present whenever either holds.
    if (!browser || (!data.live && !data.judgesSettling) || !data.token) return

    const wsBase =
      typeof window !== 'undefined'
        ? `${window.location.protocol === 'https:' ? 'wss' : 'ws'}://${window.location.host}`
        : ''

    const client = new WsPlayerClient(
      data.snapshot.player_id,
      sessionCode,
      data.token,
      wsBase,
    )
    wsClient = client
    client.connect()

    const sessionWs = new WsSessionClient(sessionCode, data.token, wsBase)
    sessionClient = sessionWs
    sessionWs.connect()

    // Reconciliation: WS frames can be lost (a dropped socket, a judge event
    // that never bridged) and every lost frame used to freeze part of the
    // page until a manual reload. A slow snapshot+history poll converges the
    // feed to server truth regardless of which frame went missing.
    const reconcile = setInterval(() => {
      if (client.sessionFinished) return
      client.refreshSnapshot()
      void refreshHistory()
    }, 45_000)

    return () => {
      clearInterval(reconcile)
      client.disconnect()
      wsClient = null
      sessionWs.disconnect()
      sessionClient = null
    }
  })

  $effect(() => {
    // `data.live` is the SSR-time value and never changes, so it alone would
    // keep this ticking after the session ends. Both countdowns that read
    // `nowMs` return null once finished, so there is nothing left to drive —
    // except while judges are still settling, when the artifact-request
    // banner's deadline countdown still needs the clock.
    if (!browser) return
    const active = (data.live && !sessionFinished) || judgingInProgress
    if (!active) return
    const timer = setInterval(() => {
      nowMs = Date.now()
    }, 1000)
    return () => clearInterval(timer)
  })

  const nextProbeCountdown = $derived.by(() => {
    // No probe is coming once the session is over, so counting down to one is
    // a lie. Left alone this pins at a red "0s" forever (or, on a page opened
    // after the fact, freezes on a stale non-zero value, because the tick
    // below never starts). Mirrors `sessionCountdown`, which nulls the same way.
    if (sessionFinished) return null
    if (nextProbeAt) {
      const target = new Date(nextProbeAt).getTime()
      return Math.max(0, Math.ceil((target - nowMs) / 1000))
    }
    let minSeconds: number | null = null
    for (const probe of probes) {
      if (probe.result !== null) continue
      if (!probe.deadline_at) continue
      const deadline = new Date(probe.deadline_at).getTime()
      const remaining = Math.max(0, Math.ceil((deadline - nowMs) / 1000))
      if (minSeconds === null || remaining < minSeconds) {
        minSeconds = remaining
      }
    }
    return minSeconds
  })

  // ponytail: server-authoritative. Prefer the game-server's running_countdown
  // frame (same value ololo's TUI renders) so browser + TUI never drift on clock
  // skew. Local clock is only a sub-second fallback during WS reconnect gaps.
  const sessionCountdown = $derived.by(() => {
    if (sessionClient?.runningCountdownSecs != null) return sessionClient.runningCountdownSecs
    if (!sessionEndsAt || sessionFinished) return null
    const target = new Date(sessionEndsAt).getTime()
    return Math.max(0, Math.ceil((target - nowMs) / 1000))
  })
</script>

<svelte:head>
  <title>{data.playerName} — ololo.dev</title>
</svelte:head>

<div class="-mx-6 -mt-8 min-h-screen bg-brand-light-blue">
  <!-- Congratulations popup: live finish + all judge verdicts landed. The
       summary block is this player's own final result. -->
  <SessionCongratsModal
    open={celebration === 'shown'}
    onClose={() => (celebration = 'dismissed')}
    entries={celebrationReport?.leaderboard ?? sessionClient?.leaderboard ?? []}
    highlightPlayerId={data.snapshot.player_id}
    summary={celebrationSummary}
  />
  <!-- Extra bottom room so the fixed replay bar never covers the last row. -->
  <div class="mx-auto w-full max-w-[1206px] px-[18px] py-[48px] {replayAvailable ? 'pb-[120px]' : ''}">
    <div class="mb-6 flex items-center justify-between gap-3">
    <!-- Back link -->
    <a
      href="/s/{sessionCode}"
      class="inline-flex items-center gap-1.5 text-sm text-brand-muted transition-colors hover:text-brand-text"
    >
      <svg
        width="14"
        height="14"
        viewBox="0 0 24 24"
        fill="none"
        aria-hidden="true"
      >
        <path
          d="M19 12H5M12 19l-7-7 7-7"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
      </svg>
      Back to {sessionLinkLabel(data.snapshot.session_status).toLowerCase()}
      <span class="font-mono text-xs text-brand-muted/70">{sessionCode}</span>
    </a>

      <!-- Whole-page view switch: the focused slides, or the full record. -->
      <div
        class="inline-flex shrink-0 rounded-[8px] bg-white p-1 shadow-sm"
        role="group"
        aria-label="Page view"
      >
        {#each viewOptions as opt (opt.id)}
          <button
            type="button"
            onclick={() => setTaskView(opt.id)}
            aria-pressed={taskView === opt.id}
            data-testid="task-view-{opt.id}"
            class="rounded-[6px] px-3 py-1 text-sm font-semibold transition-colors
                   {taskView === opt.id
              ? 'bg-brand-blue text-white'
              : 'text-brand-muted hover:text-brand-text'}"
          >
            {opt.label}
          </button>
        {/each}
      </div>
    </div>

    {#if judgingInProgress}
      <!-- Session ended, judges still scoring. One compact line that updates
           live and vanishes once pendingJudges empties, handing off to the
           plain end banner. Score updates stream in via the WS meanwhile. -->
      <div
        class="mb-4 flex items-center gap-2 rounded-md border border-blue-200 bg-blue-50 px-3 py-1.5 text-xs text-blue-800"
        role="status"
        data-testid="judges-working-dialog"
      >
        <span
          class="inline-block h-3 w-3 shrink-0 animate-spin rounded-full border-2 border-blue-400 border-t-transparent"
          aria-hidden="true"
        ></span>
        <span class="font-medium">
          Judges scoring — {pendingJudges.length} left: {pendingJudges.join(', ')}
        </span>
      </div>
    {:else if showSessionEndBanner}
      <div
        class="mb-4 flex items-center justify-between gap-3 rounded-lg border px-4 py-3 text-sm font-medium
               {sessionEndedAsCancelled
          ? 'border-amber-300 bg-amber-50 text-amber-800'
          : 'border-green-300 bg-green-50 text-green-800'}"
        role="status"
        data-testid="session-end-banner"
      >
        <span>
          {sessionEndedAsCancelled
            ? cancelDetail
            : 'Session finished — results are final once the judges settle.'}
        </span>
        <button
          type="button"
          class="shrink-0 rounded px-2 py-0.5 text-xs font-semibold hover:bg-black/5"
          onclick={() => (showSessionEndBanner = false)}
        >
          Dismiss
        </button>
      </div>
    {/if}

    {#if pendingArtifacts.length > 0}
      <div class="mb-4 space-y-2">
        <PlayerArtifactBanner requests={pendingArtifacts} now={nowMs} />
      </div>
    {/if}

    <PlayerHeader
      playerName={data.playerName}
      {avatarUrl}
      {agentDisplayName}
      live={data.live}
      {sessionFinished}
      {sessionPaused}
      {sessionCountdown}
      passedTasks={passedTasksCount}
      {totalTasks}
      probesCount={probes.length}
      {nextProbeCountdown}
      {score}
      {rank}
      {completionStatus}
      judgingPending={judgingInProgress || pendingJudges.length > 0}
      {agentConnected}
    />

    {#if replayAvailable}
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

    {#if taskView === 'report'}
      <SessionReportPanel
        report={sessionReport}
        judgesSettling={data.judgesSettling}
        sessionFinished={!data.live}
        tasks={visibleTasks}
        {probesByTask}
        {judgeResultsByTask}
        {judgeStatusesByTask}
        {evaluationsByTask}
        judgeAvatars={data.judgeAvatars ?? {}}
        {sessionCode}
        playerId={data.snapshot.player_id}
        {score}
        {rank}
        {totalTasks}
        similarityAdjustment={liveSnapshot.similarity_adjustment ?? null}
      />
    {:else if taskView === 'chat'}
      <!-- The session as a conversation: ololo hands out tasks, probes run
           as tool calls, the player answers with commits / done-flags /
           artifacts, judges reply with verdicts. -->
      <PlayerTaskChat
        tasks={visibleTasks}
        {probesByTask}
        {judgeResultsByTask}
        {judgeStatusesByTask}
        {evaluationsByTask}
        {changesByTask}
        judgeAvatars={data.judgeAvatars ?? {}}
        {sessionCode}
        playerId={data.snapshot.player_id}
        playerName={data.playerName}
        {avatarUrl}
        {agentDisplayName}
        {sessionFinished}
        live={data.live}
        {replayActive}
        replayEngaged={replay.engaged}
        {score}
        {rank}
        {totalTasks}
        similarityAdjustment={liveSnapshot.similarity_adjustment ?? null}
      />
    {:else}

    {#if data.isAdmin}
      <div class="mb-2 flex items-center justify-end gap-2">
        {#if eventLogError}
          <span class="text-[11px] text-brand-muted">{eventLogError}</span>
        {/if}
        <button
          type="button"
          data-testid="download-event-log"
          onclick={() => void downloadEventLog()}
          disabled={eventLogBusy}
          class="rounded px-2 py-0.5 text-[11px] font-semibold text-brand-blue transition-colors hover:bg-brand-blue/10 disabled:opacity-40"
        >{eventLogBusy ? 'Fetching…' : 'Event log'}</button>
      </div>
    {/if}

    <PlayerSectionTabs
      tabs={sectionTabs}
      active={section}
      onSelect={(id) => (section = id)}
    />

    {#if section === 'tasks'}
      <div role="tabpanel">
        <PlayerTaskList
          {sortedTasks}
          {taskSort}
          {cycleSort}
          {probesByTask}
          {changesByTask}
          {judgeResultsByTask}
          {judgeStatusesByTask}
          {evaluationsByTask}
          {taskStatsByOrdinal}
          {isTaskPassed}
          {taskTypeProgress}
        />
      </div>
    {:else if section === 'changes'}
      <div role="tabpanel">
        <PlayerHistory
          commits={commits}
          labels={commitLabels}
          title="All changes"
          loading={historyLoading}
          error={historyError}
          onRefresh={refreshHistory}
        />
      </div>
    {:else if section === 'judges'}
      <div role="tabpanel">
        <PlayerJudgesTab
          tasks={visibleTasks}
          {judgeResultsByTask}
          {judgeStatusesByTask}
          {evaluations}
          {changesByTask}
          judgeAvatars={data.judgeAvatars ?? {}}
          sessionFinished={liveSnapshot.session_status === 'finished' ||
            liveSnapshot.session_status === 'cancelled'}
          isAdmin={data.isAdmin ?? false}
          {sessionCode}
          playerId={data.snapshot.player_id}
          similarityAdjustment={liveSnapshot.similarity_adjustment ?? null}
        />
      </div>
    {:else if section === 'memory'}
      <div role="tabpanel">
        <PlayerMemoryTab
          entries={memoryEntries}
          updatedAt={memoryUpdatedAt}
          loading={memoryLoading}
          error={memoryError}
        />
      </div>
    {:else}
      <div role="tabpanel">
        <PlayerStatsTab tasks={visibleTasks} statsByOrdinal={taskStatsByOrdinal} />
      </div>
    {/if}
    {/if}
  </div>
</div>
