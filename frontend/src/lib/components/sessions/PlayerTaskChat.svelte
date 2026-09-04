<script lang="ts">
  import { browser } from '$app/environment'
  import type {
    PlayerTaskSummaryEntry,
    PlayerProbeEntry,
    PlayerJudgeScoredPayload,
    PlayerJudgeStatusPayload,
    PlayerTaskEvaluation,
    PlayerArtifactRef,
    PlayerHistoryCommit,
    PlayerCompletionStatus,
  } from '$lib/types/arena'
  import TypewriterMarkdown from './TypewriterMarkdown.svelte'
  import MarkdownContent from '$lib/components/MarkdownContent.svelte'
  import * as HoverCard from '$lib/components/ui/hover-card'
  import SessionSummaryCard from '$lib/components/sessions/SessionSummaryCard.svelte'
  import { buildSessionSummary, pointsChip } from '$lib/sessions/session-summary'
  import PlayerTodoChecklist from './PlayerTodoChecklist.svelte'
  import ImageLightbox from './ImageLightbox.svelte'
  import { fileName, galleryEntries, type GalleryEntry } from '$lib/sessions/artifacts'
  import { ikAvatar } from '$lib/imagekit'
  import { actualOf, expectedOf, probeBriefing, probeStatus } from '$lib/sessions/probe-briefing'
  import CheckProbeDetails from './CheckProbeDetails.svelte'
  import CopyForAgentButton from '$lib/components/CopyForAgentButton.svelte'

  type Props = {
    /** Tasks in display order; only revealed tasks should be passed in. */
    tasks: PlayerTaskSummaryEntry[]
    probesByTask: Map<string, PlayerProbeEntry[]>
    judgeResultsByTask: Map<string, PlayerJudgeScoredPayload[]>
    judgeStatusesByTask: Map<string, PlayerJudgeStatusPayload[]>
    evaluationsByTask: Map<string, PlayerTaskEvaluation>
    changesByTask: Map<string, { commits: PlayerHistoryCommit[]; mode: 'range' | 'per-commit' }>
    /** judge_slug → avatar url, from the project's judges list. */
    judgeAvatars?: Record<string, string>
    sessionCode: string
    playerId: string
    playerName: string
    avatarUrl?: string | null
    agentDisplayName?: string | null
    sessionFinished?: boolean
    /** Copy/paste validation result from finish, if the scan ran. */
    similarityAdjustment?: {
      note: string
      point_delta: number
      duplicated_pct?: number
      sources?: { join_code: string; player: string; matched_lines: number }[]
    } | null
    live?: boolean
    /** True while a replay is mid-flight: the finish line and summary card
     *  are the session's ending, so they stay hidden until the playhead
     *  reaches it. */
    replayActive?: boolean
    /** True whenever a replay is engaged (including once it settles at the
     *  end). Drives the follow-along auto-scroll. */
    replayEngaged?: boolean
    /** Final standing for the summary card (shown once the session ends). */
    score?: number
    rank?: number
    totalTasks?: number
    /** When the scheduler dispatches the next probe — the status footer's
     *  countdown. Null between tasks and once the player is done. */
    nextProbeAt?: string | null
    sessionPaused?: boolean
    /** Whether the player's ololo agent socket is up; null when unknown. */
    agentConnected?: boolean | null
    /** The player's overall standing: in progress, waiting on judges, done. */
    completionStatus?: PlayerCompletionStatus | null
  }
  let {
    tasks,
    probesByTask,
    judgeResultsByTask,
    judgeStatusesByTask,
    evaluationsByTask,
    changesByTask,
    judgeAvatars = {},
    sessionCode,
    playerId,
    playerName,
    avatarUrl = null,
    agentDisplayName = null,
    sessionFinished = false,
    similarityAdjustment = null,
    live = false,
    replayActive = false,
    replayEngaged = false,
    score = 0,
    rank = 0,
    totalTasks = 0,
    nextProbeAt = null,
    sessionPaused = false,
    agentConnected = null,
    completionStatus = null,
  }: Props = $props()

  // ── The transcript ───────────────────────────────────────────────────────
  // The session retold as a conversation. ololo hands out tasks and runs its
  // checks; the player answers with code deliveries, done-notes and captures;
  // judges ask for evidence and reply with verdicts. The raw feeds are far
  // noisier than a conversation — probes re-poll the same test, screencast
  // frames sync as dozens of `artifact:` commits — so every group below
  // collapses repetition into one message that carries its latest state.
  type ChatItem =
    | { kind: 'task'; key: string; ordinal: number; at: number | null; task: PlayerTaskSummaryEntry }
    | {
        kind: 'check'
        key: string
        ordinal: number
        at: number | null
        latest: PlayerProbeEntry
        /** Every run of this check, oldest first — the hover card pages them. */
        attempts: PlayerProbeEntry[]
        runs: number
        points: number
        /** Quiz probes ask their question via the probe URL — quote it. */
        question: string | null
      }
    | { kind: 'requests'; key: string; ordinal: number; at: number | null; requests: RequestRow[] }
    | { kind: 'commit'; key: string; ordinal: number; at: number | null; commit: PlayerHistoryCommit; title: string }
    | { kind: 'done-note'; key: string; ordinal: number; at: number | null; path: string; text: string }
    | {
        kind: 'answer'
        key: string
        ordinal: number
        at: number | null
        task: PlayerTaskSummaryEntry
        evaluation: PlayerTaskEvaluation | null
      }
    | { kind: 'closed'; key: string; ordinal: number; at: number | null; status: string }
    | { kind: 'retry'; key: string; ordinal: number; at: number | null }
    | {
        /** Open-ended task guidance while the player works: what "done"
         *  means. Once the task is delivered, the status footer narrates
         *  where it stands (judges reviewing, evidence outstanding,
         *  wrapping up). */
        kind: 'completion'
        key: string
        ordinal: number
        at: number | null
        doneFile: string | null
      }
    | { kind: 'artifact'; key: string; ordinal: number; at: number | null; entries: GalleryEntry[] }
    | {
        kind: 'judges'
        key: string
        ordinal: number
        at: number | null
        verdicts: PlayerJudgeScoredPayload[]
        failed: PlayerJudgeStatusPayload[]
        /** Runs skipped by the account's monthly judge quota (one note, no chips). */
        quotaSkipped: number
        /** Attached but not started — waiting for the task to be delivered. */
        queued: string[]
        /** Actually evaluating right now (judge_started arrived). */
        reviewing: string[]
        /** AI rubric reviews of the delivery files — evaluation, same block. */
        rubrics: RubricReview[]
        /** Open-ended tasks are "delivered"; classic tasks just complete. */
        openEnded: boolean
      }
    | { kind: 'session-end'; key: string; ordinal: number; at: number | null }
    | { kind: 'summary'; key: string; ordinal: number; at: number | null }

  /** One AI rubric review inside the task's judges bubble. */
  type RubricReview = { key: string; at: number | null; scores: Record<string, number> }

  /** One artifact request inside the task's grouped requests bubble. */
  type RequestRow = {
    id: string
    at: number | null
    judgeSlug: string | null
    instruction: string
    path: string
    delivered: boolean
    deadlineAt: string | null
  }

  const PRIO: Record<ChatItem['kind'], number> = {
    task: 0,
    check: 1,
    requests: 1,
    commit: 1,
    'done-note': 1,
    answer: 2,
    closed: 2,
    retry: 2,
    completion: 3,
    artifact: 3,
    judges: 4,
    'session-end': 9,
    summary: 10,
  }

  // The conversation's arc inside a task: the brief, then the work (checks,
  // commits, requests, deliveries — chronological), then the verdicts.
  // Judges sometimes re-request evidence after an early scoring round, which
  // put raw-timestamp verdicts ABOVE the requests they answered — the story
  // must stay ask → deliver → evaluate, so verdicts always close the task.
  const STAGE: Record<ChatItem['kind'], number> = {
    task: 0,
    check: 1,
    requests: 1,
    commit: 1,
    'done-note': 1,
    answer: 1,
    closed: 1,
    retry: 1,
    // The status strip: always the task's last word, under the verdicts —
    // it narrates where things stand right now.
    completion: 3,
    artifact: 1,
    judges: 2,
    'session-end': 9,
    summary: 9,
  }

  function ts(value: string | null | undefined): number | null {
    if (!value) return null
    const t = new Date(value).getTime()
    return Number.isNaN(t) ? null : t
  }

  function atOf(it: ChatItem): number {
    return it.at ?? Number.MAX_SAFE_INTEGER
  }

  /** `.ololo/artifacts/<request-id>/…` → the request id, or null. */
  function requestIdOf(path: string): string | null {
    const m = /\/artifacts\/([^/]+)\//.exec(path)
    return m ? m[1] : null
  }

  function artifactUrl(probeId: string, i = 0): string {
    const base = `/api/sessions/${encodeURIComponent(sessionCode)}/players/${encodeURIComponent(playerId)}/artifacts/${encodeURIComponent(probeId)}`
    return i > 0 ? `${base}?i=${i}` : base
  }

  // Judges ask for captures through probes whose command opens with this
  // marker — in the chat they are the judge's message, not a check.
  const REQUEST_RE = /^# ARTIFACT REQUEST from ([\w-]+):\s*([^\n]+)/

  // The done-flag: the player's own words about what they built. Lives in
  // the `flag:` commit as an added markdown file; the patch carries the text.
  const DONE_FILE_RE = /^\.ololo\/[^/]*done[^/]*\.md$/

  function doneNoteOf(commit: PlayerHistoryCommit): { path: string; text: string } | null {
    for (const f of commit.files) {
      if (!DONE_FILE_RE.test(f.path) || !f.patch) continue
      const text = f.patch
        .split('\n')
        .filter((l) => l.startsWith('+') && !l.startsWith('+++'))
        .map((l) => l.slice(1))
        .join('\n')
        .trim()
      if (text) return { path: f.path, text }
    }
    return null
  }

  // Auxiliary commit traffic that is not conversation: screencast frames
  // syncing (`artifact:`), memory snapshots, session baseline/final marks.
  const COMMIT_NOISE_RE = /^(artifact|memory)(\(|:)|^ololo snapshot:/

  // The completion contract's done file, as its polling probe names it —
  // the same shape the CLI's flag watcher matches.
  const DONE_FILE_IN_CMD_RE = /\.ololo\/[\w.-]*done[\w.-]*\.md/

  function doneFileOfProbes(taskProbes: PlayerProbeEntry[]): string | null {
    for (const p of taskProbes) {
      const m = DONE_FILE_IN_CMD_RE.exec(p.test_command || p.rendered_command)
      if (m) return m[0]
    }
    return null
  }

  /** `feat(uuid): Build the core game` → `Build the core game`. */
  function commitTitle(message: string): string {
    const first = message.split('\n')[0]
    const m = /^(?:feat|wip)\(([\w-]+)\):\s*(.+)/.exec(first)
    return m ? m[2] : first
  }

  const judgeNamesBySlug = $derived.by(() => {
    const map = new Map<string, string>()
    for (const list of judgeResultsByTask.values())
      for (const j of list) map.set(j.judge_slug, j.judge_name)
    for (const list of judgeStatusesByTask.values())
      for (const s of list) map.set(s.judge_slug, s.judge_name)
    return map
  })

  function judgeName(slug: string | null): string {
    if (!slug) return 'Judges'
    return judgeNamesBySlug.get(slug) ?? slug.replace(/-/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase())
  }

  /** The current task's standing plus the judges' whereabouts across every
   *  task — what the status footer narrates. Gathered while the transcript
   *  is built, from the same facts the bubbles show. */
  type StatusFacts = {
    ordinal: number
    openEnded: boolean
    doneFile: string | null
    accepted: boolean
    judging: boolean
    pendingChecks: number
    pendingDeliveries: number
    /** Judges attached to the current task, not started yet. */
    queued: string[]
    /** Judges actually evaluating right now, by task. */
    reviewing: { ordinal: number; names: string[] }[]
    /** The newest task whose judges have all reported (none running or
     *  queued anywhere) — "judges are done with task #N". */
    judgesDoneWith: number | null
    /** The current task's probe in flight, if one is. */
    inFlight: PlayerProbeEntry | null
  }

  const built = $derived.by(() => {
    const out: ChatItem[] = []
    let facts: StatusFacts | null = null
    const reviewingAll: { ordinal: number; names: string[] }[] = []
    let anyJudgeBusy = false
    let judgesDoneWith: number | null = null
    for (const task of tasks) {
      const ord = task.ordinal
      const taskProbes = probesByTask.get(task.task_id) ?? []
      const ev = evaluationsByTask.get(task.task_id)
      const judges = judgeResultsByTask.get(task.task_id) ?? []

      const revealAt =
        ts(task.scheduler_state?.activated_at) ??
        taskProbes.reduce<number | null>((min, p) => {
          const t = ts(p.dispatched_at)
          return t !== null && (min === null || t < min) ? t : min
        }, null)
      out.push({ kind: 'task', key: `task:${task.task_id}`, ordinal: ord, at: revealAt, task })

      // Delivered captures, keyed by the request that asked for them. A
      // retried request re-delivers the same file under fresh probe ids —
      // keep the newest delivery per file name, one message per request.
      const deliveredByRequest = new Map<string, PlayerArtifactRef[]>()
      for (const a of ev?.artifacts ?? []) {
        const rid = requestIdOf(a.label) ?? a.probe_id
        const arr = deliveredByRequest.get(rid)
        if (arr) arr.push(a)
        else deliveredByRequest.set(rid, [a])
      }

      // Deadlines for requests still waiting on the participant.
      const deadlineByRequest = new Map<string, string>()
      for (const r of ev?.pending_artifacts ?? []) {
        const rid = requestIdOf(`${r.path}/`) ?? requestIdOf(r.path) ?? r.probe_id
        deadlineByRequest.set(rid, r.deadline_at)
      }

      // Probes collapse per test: the same check re-polls until it passes and
      // a chat wants one message with the latest state, not every attempt.
      const probeGroups = new Map<string, PlayerProbeEntry[]>()
      for (const p of taskProbes) {
        // Internal measurement probes have no command and nothing to say;
        // `unavailable` means the check itself could not run — that is a
        // story about infrastructure, not about the player.
        if (!p.test_command && !p.rendered_command) continue
        if (p.outcome === 'unavailable') continue
        const arr = probeGroups.get(p.adapted_test_id)
        if (arr) arr.push(p)
        else probeGroups.set(p.adapted_test_id, [p])
      }

      const requestAt = new Map<string, number | null>()
      const rubricReviews: RubricReview[] = []
      // Judge-registered extra checks that have not passed yet — each one
      // holds the task's advance, so the status banner below counts them.
      let pendingChecks = 0
      // Requests deduped by (judge, instruction): a judge re-registers the
      // same ask under a fresh request id, and the reader needs it once.
      const requestRows = new Map<string, RequestRow>()
      for (const [testId, group] of probeGroups) {
        const sorted = [...group].sort((a, b) => (ts(a.dispatched_at) ?? 0) - (ts(b.dispatched_at) ?? 0))
        const latest = sorted[sorted.length - 1]
        const req = REQUEST_RE.exec(latest.test_command || latest.rendered_command)
        if (req) {
          const at = ts(sorted[0].dispatched_at)
          requestAt.set(testId, at)
          const dedupeKey = `${req[1]}:${req[2].trim()}`
          const prev = requestRows.get(dedupeKey)
          // The header folds the ask onto one line for the shell; the
          // description is the judge's text as written.
          const row: RequestRow = {
            id: testId,
            at,
            judgeSlug: req[1],
            instruction: latest.description?.trim() || req[2].trim(),
            path: `.ololo/artifacts/${testId}/`,
            delivered: deliveredByRequest.has(testId),
            deadlineAt: deadlineByRequest.get(testId) ?? null,
          }
          if (!prev) {
            requestRows.set(dedupeKey, row)
          } else {
            // Keep the earliest ask; a delivery or live deadline on any
            // retry counts for the merged row.
            requestRows.set(dedupeKey, {
              ...prev,
              delivered: prev.delivered || row.delivered,
              deadlineAt: row.deadlineAt ?? prev.deadlineAt,
            })
          }
          continue
        }
        const points = group.reduce((sum, p) => sum + p.point_delta, 0)
        // An AI rubric review is evaluation, not a check — it joins the
        // judges bubble below.
        const rubric = rubricScoresOf(latest)
        if (rubric) {
          rubricReviews.push({
            key: `rubric:${testId}`,
            at: ts(latest.resolved_at) ?? ts(latest.dispatched_at),
            scores: rubric,
          })
          continue
        }
        // A judge-registered check that is not passing yet is exactly what
        // holds the task between "verdicts are in" and the next brief — it
        // must speak (and be counted for the status banner), never sit as
        // invisible silence. Expired requests (`no_response`) let go of the
        // task and stop counting.
        const judgeRegistered = (latest.label ?? '').startsWith('registered:')
        if (judgeRegistered && probeStatus(latest) !== 'pass' && latest.outcome !== 'no_response')
          pendingChecks += 1
        // A check still failing with nothing scored is the system waiting on
        // the player ("not-done: … is missing"), not a conversation event —
        // the delivery and closure messages tell that story. Failures that
        // cost points, passes, in-flight checks and judge-registered checks
        // all still speak.
        if (probeStatus(latest) === 'fail' && points === 0 && !judgeRegistered) continue
        out.push({
          kind: 'check',
          key: `check:${task.task_id}:${testId}`,
          ordinal: ord,
          at: ts(latest.resolved_at) ?? ts(latest.dispatched_at),
          latest,
          attempts: sorted,
          runs: group.length,
          points,
          question: questionOf(latest),
        })
      }

      // Commits: only the ones that carry the player's actual work. The
      // `flag:` commit becomes the player's text message (the done-note);
      // frame syncs and memory snapshots are machine traffic, not chat.
      for (const c of changesByTask.get(task.task_id)?.commits ?? []) {
        const first = c.message.split('\n')[0]
        if (COMMIT_NOISE_RE.test(first)) continue
        const note = doneNoteOf(c)
        if (note) {
          out.push({
            kind: 'done-note',
            key: `note:${c.sha}`,
            ordinal: ord,
            at: ts(c.author_time),
            path: note.path,
            text: note.text,
          })
          // A `flag:` commit that only carries the note is fully told by the
          // message above; if it bundled code too, keep the commit as well.
          if (first.startsWith('flag:') && c.files.every((f) => DONE_FILE_RE.test(f.path))) continue
        }
        if (first.startsWith('flag:')) {
          const codeFiles = c.files.filter((f) => !DONE_FILE_RE.test(f.path))
          if (codeFiles.length === 0) continue
          out.push({
            kind: 'commit',
            key: `commit:${c.sha}`,
            ordinal: ord,
            at: ts(c.author_time),
            commit: { ...c, files: codeFiles },
            title: commitTitle(c.message),
          })
          continue
        }
        out.push({
          kind: 'commit',
          key: `commit:${c.sha}`,
          ordinal: ord,
          at: ts(c.author_time),
          commit: c,
          title: commitTitle(c.message),
        })
      }

      if (task.result && task.result.status !== 'pending') {
        // Delivering, answering, skipping — the player speaks. A task the
        // server closed unfinished (deadline, session end) is ololo's line —
        // but only once the scheduler has actually let go: a failed result
        // with live scheduler state means a retry is coming, not a closure.
        if (PLAYER_RESULT_STATUSES.has(task.result.status)) {
          out.push({
            kind: 'answer',
            key: `answer:${task.task_id}`,
            ordinal: ord,
            at: ts(task.result.evaluated_at),
            task,
            evaluation: ev ?? null,
          })
        } else if (task.scheduler_state === null || sessionFinished) {
          out.push({
            kind: 'closed',
            key: `closed:${task.task_id}`,
            ordinal: ord,
            at: ts(task.result.evaluated_at),
            status: task.result.status,
          })
        } else {
          out.push({
            kind: 'retry',
            key: `retry:${task.task_id}`,
            ordinal: ord,
            at: ts(task.result.evaluated_at),
          })
        }
      }

      // Requests, one bubble per task.
      if (requestRows.size > 0) {
        const rows = [...requestRows.values()].sort(
          (a, b) => (a.at ?? Number.MAX_SAFE_INTEGER) - (b.at ?? Number.MAX_SAFE_INTEGER),
        )
        out.push({
          kind: 'requests',
          key: `requests:${task.task_id}`,
          ordinal: ord,
          at: rows[0].at,
          requests: rows,
        })
      }

      // One delivery message per request. Deliveries carry no timestamp of
      // their own: answer their request directly, or failing that, sit just
      // before the verdicts that read them.
      const firstVerdictAt = judges.reduce<number | null>((min, j) => {
        const t = ts(j.created_at)
        return t !== null && (min === null || t < min) ? t : min
      }, null)
      for (const [rid, refs] of deliveredByRequest) {
        const byName = new Map<string, PlayerArtifactRef>()
        for (const a of refs) byName.set(fileName(a.label), a)
        const entries: GalleryEntry[] = [...byName.values()].flatMap((a) =>
          galleryEntries(a, artifactUrl),
        )
        if (entries.length > 0) {
          const reqAt = requestAt.get(rid)
          out.push({
            kind: 'artifact',
            key: `artifact:${rid}`,
            ordinal: ord,
            at: reqAt != null ? reqAt + 1 : firstVerdictAt !== null ? firstVerdictAt - 1 : null,
            entries,
          })
        }
      }

      // Every judge's word on the task in ONE bubble: verdict chips (with
      // the feedback on hover), failed judges greyed out, still-working
      // judges as pulsing chips.
      const statuses = judgeStatusesByTask.get(task.task_id) ?? []
      // Quota-denied runs collapse into one note in the bubble — the same
      // treatment the Judges tab gives them.
      const isQuota = (s: PlayerJudgeStatusPayload) =>
        (s.error ?? '').includes('Monthly judge-run limit')
      const failedAll = statuses.filter((s) => s.status === 'failed')
      const quotaSkipped = failedAll.filter(isQuota).length
      const failed = failedAll.filter((s) => !isQuota(s))
      // Queued (attached, waiting for the delivery) and reviewing (a
      // judge_started frame arrived) are different stories: only the latter
      // deserves a spinner.
      const queuedNames = [
        ...new Set(statuses.filter((s) => s.status === 'pending').map((s) => s.judge_name)),
      ]
      const reviewingNames = [
        ...new Set(statuses.filter((s) => s.status === 'running').map((s) => s.judge_name)),
      ]

      // Open-ended tasks: say plainly what "done" means and where it stands.
      // The completion contract is polled by a probe that watches the done
      // file — surface that as guidance, not as failing checks. Classic
      // probe-driven tasks (extreme startup, golf) have neither an
      // evaluation nor a done file, so no guidance line appears for them.
      const doneFile = doneFileOfProbes(taskProbes)
      const openEnded = evaluationsByTask.has(task.task_id) || doneFile !== null
      // The scheduler saying "judging" IS acceptance: the completion
      // contract has been taken and only the panel's work is outstanding.
      const judging = task.scheduler_state?.state === 'judging'
      const accepted =
        task.result?.status === 'completed' || task.result?.status === 'correct' || judging
      // Only the task the player is actually on narrates its status — a
      // finished task must not keep a stale banner alive once the next
      // brief has landed.
      const isCurrent = task.task_id === tasks[tasks.length - 1]?.task_id
      const pendingDeliveries = [...requestRows.values()].filter((r) => !r.delivered).length
      if (openEnded && !sessionFinished && isCurrent && !accepted && task.scheduler_state !== null) {
        out.push({
          kind: 'completion',
          key: `completion:${task.task_id}`,
          ordinal: ord,
          at: null,
          doneFile,
        })
      }

      // The judges' whereabouts, for the status footer.
      if (reviewingNames.length > 0) reviewingAll.push({ ordinal: ord, names: reviewingNames })
      if (reviewingNames.length > 0 || queuedNames.length > 0) anyJudgeBusy = true
      else if (judges.length > 0 || failed.length > 0) judgesDoneWith = ord
      if (isCurrent) {
        // A check dispatched and not yet answered — the newest one speaks.
        const inFlight = taskProbes
          .filter(
            (p) =>
              p.state === 'dispatched' &&
              p.outcome === null &&
              !!(p.test_command || p.rendered_command) &&
              p.rendered_command !== 'llm:rubric' &&
              !REQUEST_RE.test(p.test_command || p.rendered_command),
          )
          .sort((a, b) => (ts(b.dispatched_at) ?? 0) - (ts(a.dispatched_at) ?? 0))[0]
        facts = {
          ordinal: ord,
          openEnded,
          doneFile,
          accepted,
          judging,
          pendingChecks,
          pendingDeliveries,
          queued: queuedNames,
          reviewing: [],
          judgesDoneWith: null,
          inFlight: inFlight ?? null,
        }
      }

      if (
        judges.length > 0 ||
        failed.length > 0 ||
        quotaSkipped > 0 ||
        queuedNames.length > 0 ||
        reviewingNames.length > 0 ||
        rubricReviews.length > 0
      ) {
        const lastVerdictAt = judges.reduce<number | null>((max, j) => {
          const t = ts(j.created_at)
          return t !== null && (max === null || t > max) ? t : max
        }, null)
        out.push({
          kind: 'judges',
          key: `judges:${task.task_id}`,
          ordinal: ord,
          at: lastVerdictAt ?? rubricReviews[rubricReviews.length - 1]?.at ?? null,
          verdicts: judges,
          failed,
          quotaSkipped,
          queued: queuedNames,
          reviewing: reviewingNames,
          rubrics: rubricReviews,
          openEnded,
        })
      }
    }
    // The finish line and summary card are the ending; while a replay is still
    // sweeping they'd give it away, so hold them until the playhead arrives.
    if (sessionFinished && tasks.length > 0 && !replayActive) {
      out.push({ kind: 'session-end', key: 'session-end', ordinal: Number.MAX_SAFE_INTEGER, at: null })
      out.push({ kind: 'summary', key: 'summary', ordinal: Number.MAX_SAFE_INTEGER, at: null })
    }
    if (facts !== null) {
      const f: StatusFacts = facts
      f.reviewing = reviewingAll
      f.judgesDoneWith = anyJudgeBusy ? null : judgesDoneWith
    }
    return {
      items: out.sort(
        (a, b) =>
          a.ordinal - b.ordinal ||
          STAGE[a.kind] - STAGE[b.kind] ||
          atOf(a) - atOf(b) ||
          PRIO[a.kind] - PRIO[b.kind],
      ),
      facts,
    }
  })
  const items = $derived(built.items)
  const statusFacts = $derived(built.facts)

  // ── The status footer ────────────────────────────────────────────────────
  // What is happening right now and what comes next, pinned under the
  // transcript while the session runs: the judges at work, the check in
  // flight, the countdown to the next one. The same narration ololo's TUI
  // keeps in its chat pane's bottom row — a player should never wonder
  // whether the silence is the system working or the system stuck.
  type LiveStatus = {
    text: string
    /** Seconds to the next check, worn as a chip after the text. */
    countdown: number | null
    /** Something is actively happening — spinner instead of a still dot. */
    busy: boolean
    tone: 'info' | 'ok' | 'warn'
  }

  let nowMs = $state(Date.now())
  $effect(() => {
    if (!browser || sessionFinished || replayEngaged) return
    const timer = setInterval(() => {
      nowMs = Date.now()
    }, 1000)
    return () => clearInterval(timer)
  })

  /** `A` · `A and B` · `A, B and C`. */
  function listNames(names: string[]): string {
    if (names.length <= 1) return names[0] ?? ''
    return `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`
  }

  /** `Correctness and Data reviewing task #2, Creativity reviewing task #1`. */
  function reviewingPhrase(groups: { ordinal: number; names: string[] }[]): string {
    return groups.map((g) => `${listNames(g.names)} reviewing task #${g.ordinal}`).join(', ')
  }

  /** The judge hold spelled out: what still keeps the task open. */
  function holdPhrase(f: StatusFacts): string {
    if (f.pendingChecks > 0 && f.pendingDeliveries > 0)
      return `the judges are waiting on ${f.pendingChecks} extra ${f.pendingChecks === 1 ? 'check' : 'checks'} (retried automatically) and ${f.pendingDeliveries} requested ${f.pendingDeliveries === 1 ? 'file' : 'files'}.`
    if (f.pendingDeliveries > 0)
      return `the judges are waiting on ${f.pendingDeliveries} requested ${f.pendingDeliveries === 1 ? 'file' : 'files'} — see the request above.`
    return `${f.pendingChecks === 1 ? 'an extra check' : `${f.pendingChecks} extra checks`} from the judges ${f.pendingChecks === 1 ? 'is' : 'are'} still running; ololo retries ${f.pendingChecks === 1 ? 'it' : 'them'} automatically.`
  }

  const liveStatus = $derived.by((): LiveStatus | null => {
    if (sessionFinished || replayEngaged || tasks.length === 0) return null
    if (sessionPaused)
      return {
        tone: 'warn',
        busy: false,
        countdown: null,
        text: 'Session paused — checks and judging resume when the host unpauses.',
      }
    if (agentConnected === false)
      return {
        tone: 'warn',
        busy: false,
        countdown: null,
        text: 'Waiting for your ololo agent to reconnect — checks run on your machine, so nothing moves until it is back.',
      }
    const f = statusFacts
    const reviewing = f?.reviewing ?? []
    const working = reviewing.length > 0
    if (completionStatus === 'completed' || completionStatus === 'awaiting_judges') {
      return working
        ? {
            tone: 'info',
            busy: true,
            countdown: null,
            text: `All your tasks are done — ${reviewingPhrase(reviewing)}. The session ends when every player finishes.`,
          }
        : {
            tone: 'ok',
            busy: false,
            countdown: null,
            text: 'All your tasks are done ✓ — the session ends when every player finishes.',
          }
    }
    if (!f) return null
    // The lead: where the judges are, said before what comes next.
    const lead = working
      ? `Evaluation in progress — ${reviewingPhrase(reviewing)}.`
      : f.judgesDoneWith !== null
        ? `Judges are done with task #${f.judgesDoneWith}.`
        : ''
    const withLead = (now: string) => (lead ? `${lead} ${now}` : now)
    // A delivered open-ended task: the ball is with the panel.
    if (f.openEnded && f.accepted) {
      if (working || f.queued.length > 0) {
        const who = working ? reviewingPhrase(reviewing) : `judges queued: ${listNames(f.queued)}`
        return { tone: 'info', busy: true, countdown: null, text: `Task delivered ✓ — ${who}.` }
      }
      if (f.pendingChecks + f.pendingDeliveries > 0)
        return {
          tone: 'info',
          busy: true,
          countdown: null,
          text: `Verdicts are in — ${holdPhrase(f)} The next task starts once everything settles.`,
        }
      if (f.judging)
        return {
          tone: 'ok',
          busy: true,
          countdown: null,
          text: 'All verdicts are in ✓ — wrapping up this task; the next one is moments away.',
        }
    }
    if (f.inFlight) {
      const cmd = f.inFlight.test_command || f.inFlight.rendered_command
      const now =
        f.doneFile && DONE_FILE_IN_CMD_RE.test(cmd)
          ? `Looking for ${f.doneFile}…`
          : f.inFlight.label && !f.inFlight.label.startsWith('registered:')
            ? `Checking your code now — ${f.inFlight.label}…`
            : 'Checking your code now…'
      return { tone: 'info', busy: true, countdown: null, text: withLead(now) }
    }
    const due = nextProbeAt ? new Date(nextProbeAt).getTime() : Number.NaN
    if (!Number.isNaN(due)) {
      const secs = Math.max(0, Math.ceil((due - nowMs) / 1000))
      if (secs > 0) {
        const now =
          f.openEnded && !f.accepted && f.doneFile
            ? `ololo looks for ${f.doneFile} again in`
            : 'Next check of your code in'
        return { tone: 'info', busy: working, countdown: secs, text: withLead(now) }
      }
      return { tone: 'info', busy: true, countdown: 0, text: withLead('Next check any moment now…') }
    }
    if (lead) return { tone: working ? 'info' : 'ok', busy: working, countdown: null, text: lead }
    return null
  })

  // Follow the transcript during replay: as the playhead reveals new messages,
  // keep the newest one in view — like watching the game unfold live. Fires
  // only while a replay is engaged, so it never hijacks normal reading.
  let listEl = $state<HTMLDivElement | undefined>(undefined)
  let lastItemCount = 0
  $effect(() => {
    const n = items.length
    if (!browser || !replayEngaged || !listEl) {
      lastItemCount = n
      return
    }
    if (n === lastItemCount) return
    lastItemCount = n
    const last = listEl.lastElementChild as HTMLElement | null
    last?.scrollIntoView({ behavior: 'smooth', block: 'center' })
  })

  // ── Session summary (the card under the finish line) ─────────────────────
  const summary = $derived(
    buildSessionSummary(tasks, probesByTask, judgeResultsByTask, evaluationsByTask),
  )

  // ── Live typing ──────────────────────────────────────────────────────────
  // Messages that arrive while watching type themselves out the way AI chats
  // stream. Keys present at mount never animate; a key first seen after the
  // initial render does, once — TypewriterMarkdown samples the flag at mount.
  let seenKeys: Set<string> | null = null
  function isNewKey(key: string): boolean {
    return live && seenKeys !== null && !seenKeys.has(key)
  }
  $effect(() => {
    const keys = items.map((it) => it.key)
    if (seenKeys === null) seenKeys = new Set(keys)
    else for (const k of keys) seenKeys.add(k)
  })

  // ── Presentation helpers ─────────────────────────────────────────────────
  function timeLabel(at: number | null): string {
    if (at === null) return ''
    return new Date(at).toLocaleTimeString([], {
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    })
  }

  // Quiz probes (extreme-startup style) carry their question in the request
  // the probe fires — a `q=` URL parameter for the web contract, a `-q` flag
  // for the CLI contract. The leading qid prefix is plumbing, not question.
  function questionOf(p: PlayerProbeEntry): string | null {
    const cmd = p.rendered_command || p.test_command
    const m =
      /--data-urlencode\s+["']q=([^"']+)["']/.exec(cmd) ??
      /\s-q\s+["']([^"']+)["']/.exec(cmd) ??
      /[?&]q=([^"'\s&]+)/.exec(cmd)
    if (!m) return null
    const q = m[1].replace(/^[0-9a-z]{4,}:\s*/i, '').trim()
    return q.length > 0 ? q : null
  }

  // File names a request asks for — the deliverables, worth calling out.
  const REQUEST_FILE_RE = /\b[\w][\w-]*\.(?:png|jpe?g|webm|gif|mp4|webp|pdf|txt|md|json|csv)\b/gi

  function requestFiles(instruction: string): string[] {
    return [...new Set(instruction.match(REQUEST_FILE_RE) ?? [])]
  }

  /** The instruction as display markdown: delivery mechanics ("do not run
   *  git", "save into the artifact folder") are the system's business and
   *  stated separately; file names read best in bold. */
  function requestInstructionMd(instruction: string): string {
    const cleaned = instruction
      .split(/(?<=[.!?])\s+/)
      .filter((s) => !/\bgit\b|artifact folder/i.test(s))
      .join(' ')
      .trim()
    return cleaned.replace(REQUEST_FILE_RE, (m, offset: number, s: string) => {
      const before = s.slice(Math.max(0, offset - 2), offset)
      return before.includes('*') || before.includes('`') ? m : `**${m}**`
    })
  }

  // Expected values that are assertion code (`result.includes(…)`) explain
  // nothing to a reader — only literal expected answers are worth quoting.
  function humanExpected(p: PlayerProbeEntry): string | null {
    const expected = expectedOf(p)
    if (!expected) return null
    return /[()]|\bresult\b|=>/.test(expected) ? null : expected
  }

  // Judge-registered probes carry a machine label ("registered: <slug>") —
  // the reader deserves the judge's name and what the message even is.
  function checkLabel(p: PlayerProbeEntry): string | null {
    const m = /^registered:\s*([\w-]+)$/.exec(p.label ?? '')
    if (m) return `${judgeName(m[1])} judge requested this extra check`
    return p.label || null
  }

  // Analysis probes answer with tool JSON (`{"tool":"jscpd",…}`) — say what
  // the tool found instead of quoting its wire format.
  function analysisSummary(p: PlayerProbeEntry): string | null {
    const a = actualOf(p)
    if (!a || !a.trimStart().startsWith('{')) return null
    try {
      const parsed = JSON.parse(a) as Record<string, unknown>
      if (typeof parsed?.tool !== 'string') return null
      if (parsed.tool === 'jscpd') {
        const pct = Number(parsed.duplicated_pct ?? 0)
        const lines = Number(parsed.total_lines ?? 0)
        return `Code duplication check — ${pct}% duplicated across ${lines} lines`
      }
      return `${parsed.tool} analysis completed`
    } catch {
      return null
    }
  }

  // The server marks LLM-rubric probes with this literal instead of a shell
  // command; their "answer" is a JSON object of criterion → score.
  function rubricScoresOf(p: PlayerProbeEntry): Record<string, number> | null {
    if (p.rendered_command !== 'llm:rubric') return null
    try {
      const parsed: unknown = JSON.parse(actualOf(p) ?? '')
      if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return null
      const scores: Record<string, number> = {}
      for (const [k, v] of Object.entries(parsed)) {
        if (typeof v !== 'number') return null
        scores[k] = v
      }
      return Object.keys(scores).length > 0 ? scores : null
    } catch {
      return null
    }
  }

  const PLAYER_RESULT_STATUSES = new Set(['completed', 'correct', 'incorrect'])

  const ANSWER_LABELS: Record<string, string> = {
    completed: 'Task delivered',
    correct: 'Answered correctly',
    incorrect: 'Answered — not accepted',
  }

  function answerLabel(status: string): string {
    return ANSWER_LABELS[status] ?? `Task ${status}`
  }

  const CLOSED_LABELS: Record<string, string> = {
    failed: 'closed unfinished',
    skipped: 'skipped',
    expired: 'closed — time ran out',
  }

  function closedLabel(status: string): string {
    return CLOSED_LABELS[status] ?? `closed (${status})`
  }

  const playerLabel = $derived(agentDisplayName ? `${playerName} · ${agentDisplayName}` : playerName)

  // The task as agent-ready markdown: the brief plus the latest graded
  // check, so a player can hand the whole situation to their agent.
  function taskCopyText(task: PlayerTaskSummaryEntry): string {
    const graded = (probesByTask.get(task.task_id) ?? [])
      .filter((p) => {
        const cmd = p.test_command || p.rendered_command
        return cmd && p.rendered_command !== 'llm:rubric' && !REQUEST_RE.test(cmd)
      })
      .sort((a, b) => (ts(a.dispatched_at) ?? 0) - (ts(b.dispatched_at) ?? 0))
    const latest = graded[graded.length - 1] ?? null
    const label =
      latest === null
        ? ''
        : probeStatus(latest) === 'pass'
          ? 'passed'
          : probeStatus(latest) === 'fail'
            ? 'failed'
            : 'running'
    return probeBriefing(
      { ordinal: task.ordinal, title: task.title, content: task.content || task.adapted_content },
      latest,
      label,
    )
  }

  // Long briefs clamp; the chat is a feed, not a spec sheet.
  const BRIEF_CLAMP = 520
  let expandedBriefs = $state(new Set<string>())
  function toggleBrief(taskId: string) {
    const next = new Set(expandedBriefs)
    if (next.has(taskId)) next.delete(taskId)
    else next.add(taskId)
    expandedBriefs = next
  }

  let openCommits = $state(new Set<string>())
  function toggleCommit(sha: string) {
    const next = new Set(openCommits)
    if (next.has(sha)) next.delete(sha)
    else next.add(sha)
    openCommits = next
  }

  let lightbox = $state<{ src: string; label: string } | null>(null)

  // Follow the conversation while it is live: when new messages append and
  // the reader is already near the bottom, keep them there.
  let itemCount = $state(0)
  $effect(() => {
    const n = items.length
    if (!browser || !live) {
      itemCount = n
      return
    }
    if (n > itemCount) {
      const nearBottom =
        window.innerHeight + window.scrollY >= document.documentElement.scrollHeight - 320
      if (nearBottom) window.scrollTo({ top: document.documentElement.scrollHeight })
    }
    itemCount = n
  })

  const FILE_STATUS_COLORS: Record<string, string> = {
    added: 'text-green-600',
    deleted: 'text-brand-red',
    modified: 'text-brand-text/70',
  }
</script>

{#snippet playerAvatar()}
  {#if avatarUrl}
    <img src={ikAvatar(avatarUrl, 28)} alt="" class="mb-5 h-7 w-7 shrink-0 rounded-full object-cover shadow-sm" />
  {:else}
    <span class="mb-5 inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-brand-blue text-[12px] font-bold text-white shadow-sm" aria-hidden="true">{playerName.slice(0, 1).toUpperCase()}</span>
  {/if}
{/snippet}

<div
  data-testid="task-chat"
  class="mx-auto mt-6 flex w-full max-w-[860px] flex-col gap-4"
  role="log"
  aria-label="Session conversation"
  bind:this={listEl}
>
  {#if tasks.length === 0}
    <p class="py-10 text-center text-sm text-brand-muted">No tasks revealed yet.</p>
  {/if}

  {#each items as item (item.key)}
    {#if item.kind === 'task'}
      <!-- ololo hands out the task -->
      <!-- The brief is `content`; adapted_content carries the per-player
           adapted CHECK for open-ended tasks, which is not a message. -->
      {@const long = (item.task.content || item.task.adapted_content).length > BRIEF_CLAMP}
      {@const expanded = expandedBriefs.has(item.task.task_id)}
      <div class="flex items-end gap-2.5" data-testid="chat-task-{item.task.ordinal}">
        <img src="/logo.svg" alt="" class="mb-5 h-7 w-7 shrink-0 rounded-full bg-white p-1 shadow-sm" />
        <div class="min-w-0 max-w-[85%]">
          <div class="mb-1 flex items-baseline gap-2 pl-1 text-[11px] text-brand-muted">
            <span class="font-semibold text-brand-text/80">ololo</span>
            {#if item.at !== null}<span>{timeLabel(item.at)}</span>{/if}
          </div>
          <div class="rounded-2xl rounded-bl-md bg-white px-4 py-3 shadow-sm">
            <div class="mb-1.5 flex flex-wrap items-center gap-2">
              <span class="text-[12px] font-bold uppercase tracking-wider text-brand-blue">
                Task #{item.task.ordinal}
              </span>
              <span class="text-[14px] font-semibold text-brand-text">{item.task.title}</span>
              <span class="ml-auto flex items-center gap-2">
                {#if item.task.total_points !== undefined && item.task.total_points !== 0}
                  <span
                    class="rounded-full px-2 py-0.5 text-[11px] font-bold tabular-nums {item
                      .task.total_points > 0
                      ? 'bg-green-100 text-green-700'
                      : 'bg-red-100 text-brand-red'}"
                  >
                    {pointsChip(item.task.total_points)} pts
                  </span>
                {/if}
                <CopyForAgentButton
                  compact
                  text={taskCopyText(item.task)}
                  testid="chat-copy-task-{item.task.ordinal}"
                />
              </span>
            </div>
            <div class="relative {long && !expanded ? 'max-h-44 overflow-hidden' : ''}">
              <TypewriterMarkdown
                value={item.task.content || item.task.adapted_content}
                animate={isNewKey(item.key)}
                class="text-[13px]"
              />
              {#if long && !expanded}
                <div class="pointer-events-none absolute inset-x-0 bottom-0 h-12 bg-gradient-to-t from-white"></div>
              {/if}
            </div>
            {#if long}
              <button
                type="button"
                class="mt-1 text-[12px] font-semibold text-brand-blue hover:underline"
                onclick={() => toggleBrief(item.task.task_id)}
              >
                {expanded ? 'Show less' : 'Read the full brief'}
              </button>
            {/if}
          </div>
        </div>
      </div>
    {:else if item.kind === 'check'}
      <!-- ololo runs its check — one message per test, latest state, no
           forensics: Expected/Got internals live in the Details view. -->
      {@const status = probeStatus(item.latest)}
      {@const answer = actualOf(item.latest)}
      {@const expected = status === 'fail' ? humanExpected(item.latest) : null}
      <div class="flex items-end gap-2.5" data-testid="chat-check-{item.latest.adapted_test_id}">
        <img src="/logo.svg" alt="" class="mb-5 h-7 w-7 shrink-0 rounded-full bg-white p-1 shadow-sm" />
        <div class="min-w-0 max-w-[85%]">
          <div class="mb-1 flex items-baseline gap-2 pl-1 text-[11px] text-brand-muted">
            <span class="font-semibold text-brand-text/80">ololo</span>
            <span
              class="rounded bg-brand-light-blue px-1.5 py-px text-[10px] font-semibold text-brand-blue"
              title="An automated check ololo ran against the delivery">check</span
            >
            {#if item.at !== null}<span>{timeLabel(item.at)}</span>{/if}
          </div>
          <HoverCard.Root openDelay={150}>
            <HoverCard.Trigger class="block cursor-default no-underline">
              <div
                class="overflow-hidden rounded-2xl rounded-bl-md border bg-white shadow-sm {status === 'fail'
                  ? 'border-brand-red/25'
                  : status === 'pass'
                    ? 'border-green-300/50'
                    : 'border-transparent'}"
              >
                {#if item.question || checkLabel(item.latest) || item.latest.description}
                  <!-- What this check is about: the quiz question it asked, or
                       the test's own title and the author's explanation of
                       what it verifies (from the task definition). -->
                  <div class="border-b border-brand-border/40 px-4 pb-2 pt-2.5">
                    {#if item.question || checkLabel(item.latest)}
                      <p class="text-[12px] italic text-brand-text/70">
                        {item.question ?? checkLabel(item.latest)}
                      </p>
                    {/if}
                    {#if item.latest.description}
                      <p
                        class="mt-0.5 line-clamp-2 text-[11px] leading-snug text-brand-muted"
                        data-testid="chat-check-desc-{item.latest.adapted_test_id}"
                      >
                        {item.latest.description}
                      </p>
                    {/if}
                  </div>
                {/if}
                <div class="flex w-full items-center gap-2 px-4 py-2.5">
                  {#if status === 'pending'}
                    <span
                      class="inline-block h-3.5 w-3.5 shrink-0 animate-spin rounded-full border-2 border-brand-blue/50 border-t-transparent"
                      aria-hidden="true"
                    ></span>
                  {:else}
                    <span
                      class="inline-flex h-4 w-4 shrink-0 items-center justify-center rounded-full text-[10px] font-bold text-white {status ===
                      'pass'
                        ? 'bg-green-500'
                        : 'bg-brand-red'}"
                      aria-hidden="true">{status === 'pass' ? '✓' : '✕'}</span
                    >
                  {/if}
                  <span class="min-w-0 flex-1 truncate text-[13px] text-brand-text">
                    {#if analysisSummary(item.latest)}
                      {analysisSummary(item.latest)}
                    {:else if answer && expected}
                      {answer} <span class="text-brand-muted">— expected</span> {expected}
                    {:else if answer}
                      {answer}
                    {:else if status === 'pending'}
                      Checking…
                    {:else}
                      {status === 'pass' ? 'Check passed' : 'Check failed'}
                    {/if}
                  </span>
                  {#if item.runs > 1}
                    <span class="shrink-0 text-[11px] text-brand-muted">×{item.runs}</span>
                  {/if}
                  {#if item.points !== 0}
                    <span
                      class="shrink-0 text-[11px] font-bold tabular-nums {item.points > 0
                        ? 'text-green-600'
                        : 'text-brand-red'}"
                    >
                      {pointsChip(item.points)}
                    </span>
                  {/if}
                </div>
              </div>
            </HoverCard.Trigger>
            <!-- The full story of the check: every run pageable, the command
                 it ran, the exact expected/got values the bubble truncates,
                 the fixtures it was rendered with, and the run's vitals. -->
            <HoverCard.Content class="w-[26rem] max-w-[92vw]" align="start" data-testid="check-hover-{item.latest.adapted_test_id}">
              <CheckProbeDetails attempts={item.attempts} question={item.question} points={item.points} />
            </HoverCard.Content>
          </HoverCard.Root>
        </div>
      </div>
    {:else if item.kind === 'requests'}
      <!-- The judges' evidence requests for this task, one bubble -->
      <div class="flex items-end gap-2.5" data-testid="chat-requests-{item.ordinal}">
        <span class="mb-5 inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-amber-100 text-[13px] shadow-sm" aria-hidden="true">⚖️</span>
        <div class="min-w-0 max-w-[85%]">
          <div class="mb-1 flex items-baseline gap-2 pl-1 text-[11px] text-brand-muted">
            <span class="font-semibold text-brand-text/80">Judges</span>
            <span class="rounded bg-amber-100 px-1.5 py-px text-[10px] font-semibold text-amber-700">
              artifact {item.requests.length === 1 ? 'request' : 'requests'}
            </span>
            {#if item.at !== null}<span>{timeLabel(item.at)}</span>{/if}
          </div>
          <div class="divide-y divide-amber-200/60 rounded-2xl rounded-bl-md border border-amber-200 bg-amber-50">
            {#each item.requests as req (req.id)}
              {@const files = requestFiles(req.instruction)}
              <div class="px-4 py-2.5" data-testid="chat-request-{req.id}">
                <div class="flex items-start gap-2">
                  <div class="min-w-0 flex-1">
                    <p class="mb-0.5 text-[12px] font-semibold text-brand-text">{judgeName(req.judgeSlug)}</p>
                    <MarkdownContent value={requestInstructionMd(req.instruction)} class="text-[13px]" />
                  </div>
                  <span class="shrink-0">
                    {#if req.delivered}
                      <span class="rounded-full bg-green-100 px-2 py-0.5 text-[11px] font-semibold text-green-700">delivered ✓</span>
                    {:else if req.deadlineAt}
                      <span class="rounded-full bg-amber-100 px-2 py-0.5 text-[11px] font-semibold text-amber-700">
                        before {timeLabel(ts(req.deadlineAt))}
                      </span>
                    {:else}
                      <span class="rounded-full bg-brand-border/40 px-2 py-0.5 text-[11px] font-semibold text-brand-muted">not delivered</span>
                    {/if}
                  </span>
                </div>
                <!-- The deliverable, spelled out: which files, where. -->
                <div class="mt-1.5 flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-[11px] text-brand-text/70">
                  <span class="font-semibold">Create:</span>
                  {#if files.length > 0}
                    {#each files as f (f)}
                      <code class="rounded bg-white/70 px-1 font-mono font-bold text-brand-text/90">{f}</code>
                    {/each}
                  {:else}
                    <span>the requested file{req.path ? 's' : ''}</span>
                  {/if}
                  <span>in</span>
                  <code class="font-mono text-brand-text/60">{req.path}</code>
                </div>
              </div>
            {/each}
          </div>
        </div>
      </div>
    {:else if item.kind === 'commit'}
      <!-- The player pushes code -->
      {@const open = openCommits.has(item.commit.sha)}
      <div class="flex flex-row-reverse items-end gap-2.5" data-testid="chat-commit-{item.commit.sha.slice(0, 7)}">
        {@render playerAvatar()}
        <div class="min-w-0 max-w-[85%]">
          <div class="mb-1 flex items-baseline justify-end gap-2 pr-1 text-[11px] text-brand-muted">
            <span class="font-semibold text-brand-text/80">{playerLabel}</span>
            {#if item.at !== null}<span>{timeLabel(item.at)}</span>{/if}
          </div>
          <div class="rounded-2xl rounded-br-md bg-brand-tag-bg px-4 py-2.5">
            <button
              type="button"
              class="flex w-full items-center gap-2 text-left"
              aria-expanded={open}
              onclick={() => toggleCommit(item.commit.sha)}
            >
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" class="shrink-0 text-brand-blue" aria-hidden="true">
                <circle cx="12" cy="12" r="3.5" stroke="currentColor" stroke-width="2" />
                <path d="M12 2v6.5M12 15.5V22" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
              </svg>
              <code class="shrink-0 font-mono text-[11px] text-brand-text/60">{item.commit.sha.slice(0, 7)}</code>
              <span class="min-w-0 flex-1 truncate text-[13px] font-medium text-brand-text">{item.title}</span>
              {#if item.commit.files.length > 0}
                <span class="shrink-0 text-[11px] text-brand-muted">
                  {item.commit.files.length}
                  {item.commit.files.length === 1 ? 'file' : 'files'}
                </span>
              {/if}
            </button>
            {#if open && item.commit.files.length > 0}
              <ul class="mt-2 space-y-0.5 border-t border-brand-blue/10 pt-2">
                {#each item.commit.files as f (f.path)}
                  <li class="flex items-center gap-2 font-mono text-[11px]">
                    <span class="w-3 font-bold {FILE_STATUS_COLORS[f.status] ?? 'text-brand-muted'}">
                      {f.status === 'added' ? 'A' : f.status === 'deleted' ? 'D' : 'M'}
                    </span>
                    <span class="truncate text-brand-text/80">{f.path}</span>
                  </li>
                {/each}
              </ul>
            {/if}
          </div>
        </div>
      </div>
    {:else if item.kind === 'done-note'}
      <!-- The player's own words: the done-file, as a plain text message -->
      <div class="flex flex-row-reverse items-end gap-2.5" data-testid="chat-done-note-{item.ordinal}">
        {@render playerAvatar()}
        <div class="min-w-0 max-w-[85%]">
          <div class="mb-1 flex items-baseline justify-end gap-2 pr-1 text-[11px] text-brand-muted">
            <span class="font-semibold text-brand-text/80">{playerLabel}</span>
            {#if item.at !== null}<span>{timeLabel(item.at)}</span>{/if}
          </div>
          <div class="rounded-2xl rounded-br-md bg-brand-tag-bg px-4 py-3">
            <TypewriterMarkdown value={item.text} animate={isNewKey(item.key)} class="text-[13px]" />
            <div class="mt-2 font-mono text-[10px] text-brand-text/50">{fileName(item.path)}</div>
          </div>
        </div>
      </div>
    {:else if item.kind === 'answer'}
      <!-- The delivery lands -->
      {@const passed = item.task.result?.status === 'completed' || item.task.result?.status === 'correct'}
      <div class="flex flex-row-reverse items-end gap-2.5" data-testid="chat-answer-{item.task.ordinal}">
        {@render playerAvatar()}
        <div class="min-w-0 max-w-[85%]">
          <div class="mb-1 flex items-baseline justify-end gap-2 pr-1 text-[11px] text-brand-muted">
            <span class="font-semibold text-brand-text/80">{playerLabel}</span>
            {#if item.at !== null}<span>{timeLabel(item.at)}</span>{/if}
          </div>
          <div class="rounded-2xl rounded-br-md bg-brand-tag-bg px-4 py-3">
            <div class="flex items-center gap-2">
              <span
                class="inline-flex h-4 w-4 shrink-0 items-center justify-center rounded-full text-[10px] font-bold text-white {passed
                  ? 'bg-green-500'
                  : 'bg-brand-red'}"
                aria-hidden="true">{passed ? '✓' : '✕'}</span
              >
              <span class="text-[13px] font-semibold text-brand-text">
                {answerLabel(item.task.result?.status ?? '')} — Task #{item.task.ordinal}
              </span>
            </div>
            {#if item.task.result?.submitted_answer}
              <code class="mt-2 block break-all rounded bg-white/60 px-2 py-1 font-mono text-[12px] text-brand-text/90">
                {item.task.result.submitted_answer}
              </code>
            {/if}
            {#if item.evaluation?.todo && item.evaluation.todo.total > 0}
              <div class="mt-3 rounded-[8px] bg-white/60 px-3 py-2.5">
                <PlayerTodoChecklist evaluation={item.evaluation} />
              </div>
            {/if}
          </div>
        </div>
      </div>
    {:else if item.kind === 'closed'}
      <!-- ololo closes a task the player never finished -->
      <div class="flex items-end gap-2.5" data-testid="chat-closed-{item.ordinal}">
        <img src="/logo.svg" alt="" class="mb-5 h-7 w-7 shrink-0 rounded-full bg-white p-1 shadow-sm" />
        <div class="min-w-0 max-w-[85%]">
          <div class="mb-1 flex items-baseline gap-2 pl-1 text-[11px] text-brand-muted">
            <span class="font-semibold text-brand-text/80">ololo</span>
            {#if item.at !== null}<span>{timeLabel(item.at)}</span>{/if}
          </div>
          <div class="rounded-2xl rounded-bl-md bg-white px-4 py-3 shadow-sm">
            <div class="flex items-center gap-2">
              <span
                class="inline-flex h-4 w-4 shrink-0 items-center justify-center rounded-full bg-brand-red text-[10px] font-bold text-white"
                aria-hidden="true">✕</span
              >
              <span class="text-[13px] font-semibold text-brand-text">
                Task #{item.ordinal} {closedLabel(item.status)}
              </span>
            </div>
          </div>
        </div>
      </div>
    {:else if item.kind === 'artifact'}
      <!-- The player delivers the requested captures, one message per request -->
      <div class="flex flex-row-reverse items-end gap-2.5" data-testid="chat-artifact-{item.key.slice(9)}">
        {@render playerAvatar()}
        <div class="min-w-0 max-w-[85%]">
          <div class="mb-1 flex items-baseline justify-end gap-2 pr-1 text-[11px] text-brand-muted">
            <span class="font-semibold text-brand-text/80">{playerLabel}</span>
          </div>
          <div class="rounded-2xl rounded-br-md bg-brand-tag-bg p-1.5">
            <div class="flex flex-wrap justify-end gap-1.5">
              {#each item.entries as entry (entry.key)}
                {#if entry.content_type.startsWith('image/')}
                  <button
                    type="button"
                    class="block cursor-zoom-in"
                    onclick={() => (lightbox = { src: entry.src, label: entry.label })}
                  >
                    <!-- Fixed height (not max-height): a lazy image with no
                         reserved box collapses to 0px and pushes the whole
                         transcript down when it paints — the reader's scroll
                         position jumped on every artifact load (audit UI-M4). -->
                    <img
                      src={entry.src}
                      alt={entry.label}
                      loading="lazy"
                      class="h-44 rounded-[10px] bg-black/5 object-contain"
                    />
                  </button>
                {:else if entry.content_type.startsWith('video/')}
                  <!-- svelte-ignore a11y_media_has_caption -->
                  <video src={entry.src} controls preload="metadata" class="h-44 rounded-[10px] bg-black/5"></video>
                {:else}
                  <a
                    href={entry.src}
                    download
                    class="flex items-center gap-2 px-2.5 py-1.5 text-[13px] font-medium text-brand-blue hover:underline"
                  >
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" aria-hidden="true">
                      <path d="M12 3v12m0 0l-4-4m4 4l4-4M4 21h16" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
                    </svg>
                    {entry.label}
                  </a>
                {/if}
              {/each}
            </div>
            <div class="px-2 py-1 text-right font-mono text-[10px] text-brand-text/50">
              {item.entries.length === 1 ? item.entries[0].label : `${item.entries.length} captures`}
            </div>
          </div>
        </div>
      </div>
    {:else if item.kind === 'judges'}
      <!-- The judges' word on the task, one bubble: verdict chips with the
           feedback in a hover card; failed grey; still-working pulsing. -->
      {@const total = item.verdicts.reduce((sum, v) => sum + v.point_delta, 0)}
      <div class="flex items-end gap-2.5" data-testid="chat-judges-{item.ordinal}">
        <span class="mb-5 inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-brand-light-blue text-[13px] shadow-sm" aria-hidden="true">⚖️</span>
        <div class="min-w-0 max-w-[85%]">
          <div class="mb-1 flex items-baseline gap-2 pl-1 text-[11px] text-brand-muted">
            <span class="font-semibold text-brand-text/80">Judges</span>
            <span class="rounded bg-brand-light-blue px-1.5 py-px text-[10px] font-semibold text-brand-blue">Task #{item.ordinal}</span>
            {#if item.at !== null}<span>{timeLabel(item.at)}</span>{/if}
            {#if item.verdicts.length > 0 && total !== 0}
              <span class="font-bold tabular-nums {total > 0 ? 'text-green-600' : 'text-brand-red'}">
                {pointsChip(total)} pts
              </span>
            {/if}
          </div>
          <!-- All runs quota-skipped leaves no chips at all — an empty white
               pill above the note reads as a glitch, so the container hides. -->
          {#if item.verdicts.length > 0 || item.failed.length > 0 || item.queued.length > 0 || item.reviewing.length > 0 || item.rubrics.length > 0}
          <div class="flex flex-wrap items-center gap-1.5 rounded-2xl rounded-bl-md bg-white px-3 py-2.5 shadow-sm">
            {#each item.rubrics as rubric (rubric.key)}
              <HoverCard.Root openDelay={150}>
                <HoverCard.Trigger
                  class="inline-flex cursor-default items-center gap-1.5 rounded-full border border-brand-border/60 bg-white py-0.5 pl-2 pr-2 no-underline transition-colors hover:border-brand-blue/40"
                  data-testid="chat-rubric-{item.ordinal}"
                >
                  <img src="/logo.svg" alt="" class="h-[18px] w-[18px] rounded-full bg-white p-0.5" />
                  <span class="text-[10px] font-bold uppercase tracking-wider text-brand-blue">ololo AI review</span>
                  {#each Object.entries(rubric.scores) as [criterion, score] (criterion)}
                    <span class="text-[11px] font-medium text-brand-text/80">
                      {criterion}
                      <span class="ml-0.5 font-bold tabular-nums {score >= 7 ? 'text-green-600' : score >= 4 ? 'text-amber-600' : 'text-brand-red'}">{score}</span>
                    </span>
                  {/each}
                  {#if rubric.at !== null}
                    <span class="text-[10px] tabular-nums text-brand-muted">{timeLabel(rubric.at)}</span>
                  {/if}
                </HoverCard.Trigger>
                <HoverCard.Content class="w-80" align="start">
                  <div class="mb-1.5 flex items-center gap-2">
                    <img src="/logo.svg" alt="" class="h-4 w-4 rounded-full bg-white p-0.5" />
                    <span class="text-[12px] font-semibold text-brand-text">ololo AI review</span>
                    {#if rubric.at !== null}
                      <span class="ml-auto text-[10px] text-brand-muted">{timeLabel(rubric.at)}</span>
                    {/if}
                  </div>
                  <p class="text-[12px] leading-relaxed text-brand-text/80">
                    ololo runs this review automatically the moment a delivery lands: an AI
                    model reads the done-note and the delivered files and scores them against
                    the task's rubric — the criteria on the chip.
                  </p>
                </HoverCard.Content>
              </HoverCard.Root>
            {/each}
            {#each item.verdicts as v (v.judge_slug + v.created_at)}
              <HoverCard.Root openDelay={150}>
                <HoverCard.Trigger
                  class="inline-flex cursor-default items-center gap-1.5 rounded-full border border-brand-border/60 bg-brand-light-blue/60 py-0.5 pl-1 pr-2 no-underline transition-colors hover:border-brand-blue/40"
                  data-testid="chat-judge-chip-{v.judge_slug}-{v.task_id}"
                >
                  {#if judgeAvatars[v.judge_slug]}
                    <img src={ikAvatar(judgeAvatars[v.judge_slug], 18)} alt="" class="h-[18px] w-[18px] rounded-full object-cover" />
                  {:else}
                    <span class="inline-flex h-[18px] w-[18px] items-center justify-center rounded-full bg-white text-[10px]" aria-hidden="true">⚖️</span>
                  {/if}
                  <span class="text-[12px] font-medium text-brand-text">{v.judge_name}</span>
                  <span
                    class="text-[11px] font-bold tabular-nums {v.point_delta > 0
                      ? 'text-green-600'
                      : v.point_delta < 0
                        ? 'text-brand-red'
                        : 'text-brand-muted'}">{pointsChip(v.point_delta)}</span
                  >
                </HoverCard.Trigger>
                <HoverCard.Content class="w-80" align="start">
                  <div class="mb-1.5 flex items-center gap-2">
                    <span class="text-[12px] font-semibold text-brand-text">{v.judge_name}</span>
                    <span
                      class="rounded-full px-2 py-0.5 text-[11px] font-bold tabular-nums {v.point_delta > 0
                        ? 'bg-green-100 text-green-700'
                        : v.point_delta < 0
                          ? 'bg-red-100 text-brand-red'
                          : 'bg-brand-light-blue text-brand-muted'}"
                    >
                      {pointsChip(v.point_delta)} pts
                    </span>
                    <span class="ml-auto text-[10px] text-brand-muted">{timeLabel(ts(v.created_at))}</span>
                  </div>
                  {#if v.feedback}
                    <MarkdownContent value={v.feedback} class="text-[12px]" />
                  {:else}
                    <p class="text-[12px] text-brand-muted">No comment left.</p>
                  {/if}
                </HoverCard.Content>
              </HoverCard.Root>
            {/each}
            {#each item.failed as s (s.judge_slug)}
              <HoverCard.Root openDelay={150}>
                <HoverCard.Trigger
                  class="inline-flex cursor-default items-center gap-1 rounded-full border border-brand-border/60 bg-brand-border/20 px-2 py-0.5 no-underline"
                  data-testid="chat-judge-chip-failed-{s.judge_slug}-{s.task_id}"
                >
                  <span class="text-[12px] text-brand-muted line-through">{s.judge_name}</span>
                </HoverCard.Trigger>
                <HoverCard.Content class="w-72" align="start">
                  <p class="text-[12px] text-brand-text/80">
                    {s.judge_name} could not score this task{s.error ? ` — ${s.error}` : '.'}
                  </p>
                </HoverCard.Content>
              </HoverCard.Root>
            {/each}
            {#each item.reviewing as name (name)}
              <!-- judge_started arrived: this judge is genuinely evaluating. -->
              <span
                class="inline-flex animate-pulse items-center gap-1.5 rounded-full border border-brand-blue/30 bg-white px-2 py-0.5"
                data-testid="chat-judge-chip-reviewing-{item.ordinal}-{name}"
              >
                <span class="inline-block h-2.5 w-2.5 animate-spin rounded-full border-[1.5px] border-brand-blue/50 border-t-transparent" aria-hidden="true"></span>
                <span class="text-[12px] text-brand-text/80">{name} — reviewing</span>
              </span>
            {/each}
            {#each item.queued as name (name)}
              <!-- Attached but not started: no spinner — nothing is running,
                   the panel waits for the delivery (or for a queue slot). -->
              <span
                class="inline-flex items-center gap-1.5 rounded-full border border-brand-border/50 bg-brand-border/10 px-2 py-0.5 opacity-70"
                title="Queued — starts once the task is delivered"
                data-testid="chat-judge-chip-queued-{item.ordinal}-{name}"
              >
                <span class="text-[12px] text-brand-muted">{name}</span>
              </span>
            {/each}
          </div>
          {#if item.verdicts.length === 0 && item.reviewing.length === 0 && item.queued.length > 0}
            <p class="mt-1 pl-1 text-[11px] text-brand-muted" data-testid="chat-judges-waiting-{item.ordinal}">
              {item.openEnded
                ? 'This panel reviews the task once you deliver it.'
                : 'This panel reviews the task once it completes.'}
            </p>
          {/if}
          {/if}
          {#if item.quotaSkipped > 0}
            <p
              class="mt-2 rounded-md bg-amber-50 px-3 py-2 text-[12px] text-amber-900"
              data-testid="chat-judge-quota-{item.ordinal}"
            >
              <span class="font-semibold">Reached the monthly limit of judge evaluations</span>
              — {item.quotaSkipped}
              {item.quotaSkipped === 1 ? 'review was' : 'reviews were'} skipped here.
            </p>
          {/if}
        </div>
      </div>
    {:else if item.kind === 'retry'}
      <!-- A failed attempt on a task the scheduler still holds — quiet note -->
      <div class="text-center" data-testid="chat-retry-{item.ordinal}">
        <span class="text-[11px] text-brand-muted">
          Task #{item.ordinal} attempt failed — retry scheduled
        </span>
      </div>
    {:else if item.kind === 'completion'}
      <!-- The open-ended contract, spelled out while the player works: what
           "done" means. Once delivered, the status footer takes over. -->
      <div
        class="mx-auto w-full max-w-[640px] rounded-xl border border-dashed border-brand-border/70 bg-brand-light-blue/30 px-4 py-2.5 text-center"
        data-testid="chat-completion-{item.ordinal}"
      >
        <p class="text-[13px] text-brand-text/80">
          <span class="font-semibold">You decide when this task is done.</span>
          {#if item.doneFile}
            Write <code class="rounded bg-white/80 px-1 font-mono text-[11px] font-bold">{item.doneFile}</code>
            describing what you built — ololo picks it up automatically.
          {:else}
            Declare completion the way the brief says — ololo picks it up automatically.
          {/if}
        </p>
      </div>
    {:else if item.kind === 'session-end'}
      <div class="my-2 flex items-center gap-3" data-testid="chat-session-end">
        <span class="h-px flex-1 bg-brand-border"></span>
        <span class="text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Session finished</span>
        <span class="h-px flex-1 bg-brand-border"></span>
      </div>

    {:else if item.kind === 'summary'}
      <!-- The final word: where the points came from, at a glance. The
           session report shows the same card, from the same component. -->
      <SessionSummaryCard
        {summary}
        {score}
        {rank}
        {totalTasks}
        taskCount={tasks.length}
        {similarityAdjustment}
        {judgeAvatars}
        testid="chat-summary"
      />
    {/if}
  {/each}
  {#if liveStatus}
    <!-- The status footer: what is happening now and what comes next. Sits
         under the transcript, outside the scroll, so it is always in view —
         the "typing…" line of this chat. -->
    <div class="mx-auto w-full max-w-[920px] pt-2" data-testid="chat-status" aria-live="polite">
      <div
        class="flex items-center gap-2.5 rounded-xl border px-3.5 py-2 text-[13px] shadow-sm {liveStatus.tone === 'warn'
          ? 'border-amber-200 bg-amber-50 text-amber-900'
          : liveStatus.tone === 'ok'
            ? 'border-green-200/80 bg-white text-brand-text/80'
            : 'border-brand-border/60 bg-white text-brand-text/80'}"
      >
        {#if liveStatus.busy}
          <span
            class="inline-block h-3.5 w-3.5 shrink-0 animate-spin rounded-full border-2 border-brand-blue/50 border-t-transparent"
            aria-hidden="true"
          ></span>
        {:else}
          <span
            class="inline-block h-2 w-2 shrink-0 rounded-full {liveStatus.tone === 'warn'
              ? 'bg-amber-500'
              : liveStatus.tone === 'ok'
                ? 'bg-green-500'
                : 'bg-brand-blue/60'}"
            aria-hidden="true"
          ></span>
        {/if}
        <span class="min-w-0 flex-1">{liveStatus.text}</span>
        {#if liveStatus.countdown !== null && liveStatus.countdown > 0}
          <span
            class="shrink-0 rounded-full bg-brand-light-blue px-2 py-0.5 text-[12px] font-bold tabular-nums text-brand-blue"
            data-testid="chat-status-countdown">{liveStatus.countdown}s</span
          >
        {/if}
      </div>
    </div>
  {/if}

</div>

{#if lightbox}
  <ImageLightbox src={lightbox.src} alt={lightbox.label} onclose={() => (lightbox = null)} />
{/if}
