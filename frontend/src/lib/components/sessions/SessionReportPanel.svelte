<script lang="ts">
  import MarkdownContent from "$lib/components/MarkdownContent.svelte";
  import ImageLightbox from "$lib/components/sessions/ImageLightbox.svelte";
  import SessionSummaryCard from "$lib/components/sessions/SessionSummaryCard.svelte";
  import { ikAvatar } from "$lib/imagekit";
  import { fileName, galleryEntries } from "$lib/sessions/artifacts";
  import { buildSessionSummary, isTaskPassed, pointsChip } from "$lib/sessions/session-summary";
  import type {
    PlayerArtifactRef,
    PlayerJudgeScoredPayload,
    PlayerJudgeStatusPayload,
    PlayerProbeEntry,
    PlayerSessionReport,
    PlayerTaskEvaluation,
    PlayerTaskSummaryEntry,
  } from "$lib/types/arena";

  let {
    report,
    judgesSettling = false,
    sessionFinished = false,
    tasks = [],
    probesByTask = new Map(),
    judgeResultsByTask = new Map(),
    judgeStatusesByTask = new Map(),
    evaluationsByTask = new Map(),
    judgeAvatars = {},
    sessionCode = "",
    playerId = "",
    score = 0,
    rank = 0,
    totalTasks = 0,
    similarityAdjustment = null,
  }: {
    report: PlayerSessionReport | null;
    /** Judges are still working — the report is written last, so it is coming. */
    judgesSettling?: boolean;
    sessionFinished?: boolean;
    tasks?: PlayerTaskSummaryEntry[];
    probesByTask?: Map<string, PlayerProbeEntry[]>;
    judgeResultsByTask?: Map<string, PlayerJudgeScoredPayload[]>;
    /** Every attached judge's lifecycle, so a run that never produced a
     *  verdict is named rather than silently missing. */
    judgeStatusesByTask?: Map<string, PlayerJudgeStatusPayload[]>;
    evaluationsByTask?: Map<string, PlayerTaskEvaluation>;
    judgeAvatars?: Record<string, string>;
    sessionCode?: string;
    playerId?: string;
    score?: number;
    rank?: number;
    totalTasks?: number;
    similarityAdjustment?: {
      note: string;
      point_delta: number;
      duplicated_pct?: number;
      sources?: { join_code: string; player: string; matched_lines: number }[];
    } | null;
  } = $props();

  const doc = $derived(report?.document ?? null);
  const written = $derived(report ? new Date(report.created_at).toLocaleString() : null);
  const summary = $derived(
    buildSessionSummary(tasks, probesByTask, judgeResultsByTask, evaluationsByTask),
  );

  const taskByOrdinal = $derived(new Map(tasks.map((t) => [t.ordinal, t])));

  /** The judge's note per task, so the completed list reads as a story rather
   *  than a checklist. */
  const noteByOrdinal = $derived(
    new Map((doc?.built.tasks ?? []).map((t) => [t.ordinal, t.note])),
  );

  /** Completed tasks come from the record, not the model: the page knows
   *  exactly which ones passed, and what they were worth. */
  const completed = $derived(tasks.filter(isTaskPassed).sort((a, b) => a.ordinal - b.ordinal));

  function artifactUrl(probeId: string, i = 0): string {
    const base = `/api/sessions/${encodeURIComponent(sessionCode)}/players/${encodeURIComponent(playerId)}/artifacts/${encodeURIComponent(probeId)}`;
    return i > 0 ? `${base}?i=${i}` : base;
  }


  /** Everything the session produced to look at — screenshots and screencasts
   *  the checks captured — in one strip under what was built. Deduped by file
   *  name: a re-requested capture arrives again under a fresh id. */
  const shots = $derived.by(() => {
    const byName = new Map<string, PlayerArtifactRef>();
    for (const ev of evaluationsByTask.values()) {
      for (const a of ev.artifacts ?? []) byName.set(fileName(a.label), a);
    }
    return [...byName.values()].flatMap((a) => galleryEntries(a, artifactUrl));
  });

  /** The strip's images alone: a screencast has its own fullscreen, and a
   *  downloadable file has nothing to show. Walking the gallery must not step
   *  through either of them. */
  const galleryShots = $derived(shots.filter((s) => s.content_type.startsWith("image/")));

  // Fullscreen over the gallery, by index, so ‹ › and the arrow keys walk the
  // strip without leaving it. Same lightbox the judges tab opens.
  let lightboxIndex = $state<number | null>(null);
  const lightboxShot = $derived(
    lightboxIndex === null ? null : (galleryShots[lightboxIndex] ?? null),
  );

  function openLightbox(key: string) {
    const i = galleryShots.findIndex((s) => s.key === key);
    if (i >= 0) lightboxIndex = i;
  }

  function stepLightbox(delta: number) {
    if (lightboxIndex === null || galleryShots.length === 0) return;
    const n = galleryShots.length;
    lightboxIndex = (lightboxIndex + delta + n) % n;
  }

  function taskLabel(ordinal: number): string {
    const t = taskByOrdinal.get(ordinal);
    return t ? `Task #${t.ordinal} · ${t.title}` : `Task #${ordinal}`;
  }

  // ── The panel, in full ──────────────────────────────────────────────────
  // The reporter writes two lines about each judge; the judges themselves
  // wrote the verdict, moved the points and filled the criteria sheet. The
  // report used to show only the two lines, which left the player reading
  // praise next to a score they could not account for. Both go in: the
  // reporter's summary, and the record it was summarising.

  type PanelVerdict = {
    key: string;
    verdict: PlayerJudgeScoredPayload;
    /** The reporter's two lines for this judge, when it wrote any. */
    note: { good: string; improve?: string | null } | null;
  };

  type PanelGroup = {
    task_id: string;
    ordinal: number;
    label: string;
    points: number;
    verdicts: PanelVerdict[];
  };

  /** Verdicts in the order the reporter read them — task order, then judge —
   *  so pairing its notes back onto them stays stable. */
  const orderedVerdicts = $derived.by(() => {
    const out: { ordinal: number; task_id: string; v: PlayerJudgeScoredPayload }[] = [];
    for (const t of tasks) {
      for (const v of judgeResultsByTask.get(t.task_id) ?? []) {
        out.push({ ordinal: t.ordinal, task_id: t.task_id, v });
      }
    }
    return out.sort(
      (a, b) => a.ordinal - b.ordinal || a.v.judge_slug.localeCompare(b.v.judge_slug),
    );
  });

  /** Judge verdicts grouped under the task they scored.
   *
   *  The reporter's notes are matched by judge name, in order: a judge that
   *  ran on two tasks gets two entries in the document, and the first belongs
   *  to the first task it scored. Names it invented — or judges it skipped —
   *  simply leave the summary lines off that card. */
  const panelGroups = $derived.by(() => {
    const queues = new Map<string, { good: string; improve?: string | null }[]>();
    for (const n of doc?.judges ?? []) {
      const key = n.judge.trim().toLowerCase();
      const q = queues.get(key) ?? [];
      q.push({ good: n.good, improve: n.improve });
      queues.set(key, q);
    }
    const byTask = new Map<string, PanelGroup>();
    for (const { ordinal, task_id, v } of orderedVerdicts) {
      const group = byTask.get(task_id) ?? {
        task_id,
        ordinal,
        label: taskLabel(ordinal),
        points: 0,
        verdicts: [],
      };
      group.points += v.point_delta;
      group.verdicts.push({
        key: `${task_id}:${v.judge_slug}:${v.created_at}`,
        verdict: v,
        note: queues.get(v.judge_name.trim().toLowerCase())?.shift() ?? null,
      });
      byTask.set(task_id, group);
    }
    return [...byTask.values()].sort((a, b) => a.ordinal - b.ordinal);
  });

  /** Judges that were attached but produced no verdict. A missing name reads
   *  as an oversight; naming it — and why — reads as a record. */
  const unscoredJudges = $derived.by(() => {
    const scored = new Set(orderedVerdicts.map((e) => `${e.task_id}:${e.v.judge_slug}`));
    const out: { key: string; name: string; ordinal: number; reason: string }[] = [];
    for (const t of tasks) {
      for (const st of judgeStatusesByTask.get(t.task_id) ?? []) {
        if (st.status === "scored" || scored.has(`${t.task_id}:${st.judge_slug}`)) continue;
        out.push({
          key: `${t.task_id}:${st.judge_slug}`,
          name: st.judge_name,
          ordinal: t.ordinal,
          reason: st.error ?? (st.status === "failed" ? "the run failed" : "it never ran"),
        });
      }
    }
    return out.sort((a, b) => a.ordinal - b.ordinal || a.name.localeCompare(b.name));
  });

  const verdictCount = $derived(orderedVerdicts.length);
  const judgeCount = $derived(new Set(orderedVerdicts.map((e) => e.v.judge_slug)).size);

  /** The criteria sheet, flattened for the scorecard: one row per criterion
   *  per judge that scored it, with the weight it carried. */
  type ScoreRow = {
    key: string;
    ordinal: number;
    title: string;
    weight: number;
    score: number | null;
    judge: string;
    rationale: string;
  };

  const scorecard = $derived.by(() => {
    const rows: ScoreRow[] = [];
    for (const t of tasks) {
      const ev = evaluationsByTask.get(t.task_id);
      if (!ev) continue;
      for (const c of ev.criteria ?? []) {
        for (const sc of c.scores ?? []) {
          rows.push({
            key: `${t.task_id}:${c.key}:${sc.judge_slug}`,
            ordinal: t.ordinal,
            title: c.title,
            weight: c.weight,
            score: sc.score ?? null,
            judge: sc.judge_slug,
            rationale: sc.rationale,
          });
        }
      }
    }
    return rows.sort((a, b) => a.ordinal - b.ordinal || b.weight - a.weight);
  });

  /** A judge's display name, resolved from the verdicts that carry both. */
  const judgeNameBySlug = $derived(
    new Map(orderedVerdicts.map((e) => [e.v.judge_slug, e.v.judge_name])),
  );

  function judgeLabel(slug: string): string {
    return (
      judgeNameBySlug.get(slug) ??
      slug.replace(/-/g, " ").replace(/\b\w/g, (c) => c.toUpperCase())
    );
  }

  /** Criteria are scored out of ten across every judge — the one number on
   *  this page that is comparable between them. */
  function scoreTone(score: number | null): string {
    if (score === null) return "muted";
    return score >= 7 ? "pos" : score >= 4 ? "warn" : "neg";
  }

  function pointsTone(n: number): string {
    return n > 0 ? "pos" : n < 0 ? "neg" : "muted";
  }

  /** Judges write paths as globs — `src/domain/*.ts`, `src/**` — and markdown
   *  reads those stars as emphasis: half a rationale went italic and lost the
   *  stars entirely. A star glued to a path character is escaped; the ones
   *  around *emphasis* and **bold** are left to do their job. */
  function markdownSafe(text: string): string {
    return text.replace(/\*/g, (star, i: number) =>
      text[i - 1] === "/" || text[i + 1] === "." || text[i + 1] === "/" ? "\\*" : star,
    );
  }
</script>

<!-- The session debrief: what you built, where it fought back, what each judge
     thought, and how to score higher. Laid out in the documentation's blocks —
     same type scale, same callout, same table, same counted list — because it
     is the one page here that is read rather than scanned. Deliberately not a
     verdict card: no rating, no points of its own. -->
<section class="mx-auto w-full max-w-[820px]" data-testid="session-report">
  {#if report}
    <!-- The same card the chat closes with, from the same component. -->
    <SessionSummaryCard
      {summary}
      {score}
      {rank}
      {totalTasks}
      taskCount={tasks.length}
      {similarityAdjustment}
      {judgeAvatars}
      testid="report-summary"
      inDocument
    />

    <article class="doc mt-4">
      <h2>Your session report</h2>
      <ul class="breadcrumbs">
        <li><span>{report.judge_name}</span></li>
        {#if written}<li><span>{written}</span></li>{/if}
      </ul>

      {#if doc}
        <section data-testid="report-built">
          <h3>What you built</h3>
          <p>{doc.built.brief}</p>

          {#if completed.length > 0}
            <ul class="doc-list">
              {#each completed as t (t.task_id)}
                <li data-testid="report-built-task-{t.ordinal}">
                  <b>{t.title}</b>{#if noteByOrdinal.get(t.ordinal)}{" — "}{noteByOrdinal.get(
                      t.ordinal,
                    )}{/if}
                </li>
              {/each}
            </ul>
          {/if}

          {#if shots.length > 0}
            <div class="shots" data-testid="report-artifacts">
              {#each shots as shot (shot.key)}
                {#if shot.content_type.startsWith("image/")}
                  <figure>
                    <button
                      type="button"
                      class="thumb"
                      title="{shot.label} — click to enlarge"
                      data-testid="report-artifact-{shot.key}"
                      onclick={() => openLightbox(shot.key)}
                    >
                      <img src={shot.src} alt={shot.label} loading="lazy" />
                    </button>
                    <figcaption>{shot.label}</figcaption>
                  </figure>
                {:else if shot.content_type.startsWith("video/")}
                  <figure>
                    <!-- svelte-ignore a11y_media_has_caption -->
                    <video src={shot.src} controls preload="metadata" title={shot.label}></video>
                    <figcaption>{shot.label}</figcaption>
                  </figure>
                {:else}
                  <a class="file" href={shot.src} target="_blank" rel="noreferrer">{shot.label}</a>
                {/if}
              {/each}
            </div>
          {/if}
        </section>

        {#if (doc.friction ?? []).length > 0}
          <section data-testid="report-friction">
            <h3>Where it fought back</h3>
            <!-- Index keys on purpose: this is the model's own list, and it
                 may legitimately name the same task twice (BZU2NI reported
                 two rough patches on task 1 — keyed by ordinal, the page
                 threw each_key_duplicate and never opened). The list is
                 render-once content; it never reorders. -->
            {#each doc.friction ?? [] as f, i (i)}
              <blockquote>
                <b>{taskLabel(f.ordinal)}</b>
                <p>{f.what_happened}</p>
                {#if f.why}<p>{f.why}</p>{/if}
              </blockquote>
            {/each}
          </section>
        {/if}

        {#if panelGroups.length > 0}
          <!-- One card per verdict, under the task it scored: the judge, what
               it moved, the reporter's two lines, and the verdict itself.
               The table this replaced showed the two lines alone — praise
               beside a score the player could not account for. -->
          <section data-testid="report-judges">
            <h3>Judges</h3>
            <p>
              {verdictCount}
              {verdictCount === 1 ? "evaluation" : "evaluations"} from {judgeCount}
              {judgeCount === 1 ? "judge" : "judges"}, worth
              <b class={pointsTone(summary.judgePoints)}>{pointsChip(summary.judgePoints)}</b>
              points of your score.
            </p>

            {#each panelGroups as g (g.task_id)}
              <div class="panel-task" data-testid="report-judges-task-{g.ordinal}">
                <div class="panel-task-head">
                  <b>{g.label}</b>
                  <span class="pts {pointsTone(g.points)}">{pointsChip(g.points)} pts</span>
                </div>

                {#each g.verdicts as row (row.key)}
                  <article class="verdict" data-testid="report-verdict-{row.verdict.judge_slug}">
                    <header>
                      {#if judgeAvatars[row.verdict.judge_slug]}
                        <img
                          src={ikAvatar(judgeAvatars[row.verdict.judge_slug], 28)}
                          alt=""
                          class="who-avatar"
                        />
                      {:else}
                        <span class="who-avatar fallback" aria-hidden="true">⚖️</span>
                      {/if}
                      <b class="who">{row.verdict.judge_name}</b>
                      <span class="pts {pointsTone(row.verdict.point_delta)}">
                        {pointsChip(row.verdict.point_delta)} pts
                      </span>
                    </header>

                    {#if row.note}
                      <dl class="brief">
                        <dt>Liked</dt>
                        <dd>{row.note.good}</dd>
                        <dt>Wants better</dt>
                        <dd>{row.note.improve ?? "Nothing this time."}</dd>
                      </dl>
                    {/if}

                    {#if row.verdict.feedback}
                      <div class="verdict-text" data-testid="report-verdict-text">
                        <MarkdownContent
                          value={markdownSafe(row.verdict.feedback)}
                          class="text-[15px]"
                        />
                      </div>
                    {/if}
                  </article>
                {/each}
              </div>
            {/each}

            {#if unscoredJudges.length > 0}
              <p class="muted" data-testid="report-judges-unscored">
                No verdict from {#each unscoredJudges as u, i (u.key)}{i > 0
                    ? ", "
                    : ""}{u.name} (Task #{u.ordinal} — {u.reason}){/each}.
              </p>
            {/if}
          </section>
        {:else if (doc.judges ?? []).length > 0}
          <!-- No verdicts reached this page — an older snapshot, or a report
               that outlived the runs behind it. The reporter's two lines are
               still worth showing on their own. -->
          <section data-testid="report-judges">
            <h3>Judges</h3>
            <div class="table-scroll">
              <table>
                <thead>
                  <tr>
                    <th>Judge</th>
                    <th>What was good</th>
                    <th>What should be improved</th>
                  </tr>
                </thead>
                <tbody>
                  {#each doc.judges ?? [] as j, i (i)}
                    <tr>
                      <td class="who">{j.judge}</td>
                      <td>{j.good}</td>
                      <td>{j.improve ?? "Nothing this time."}</td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          </section>
        {/if}

        {#if scorecard.length > 0}
          <!-- The criteria sheet the open-ended judges filled in: the only
               numbers on this page that compare between judges, each with the
               weight it carried into the score and the line behind it. -->
          <section data-testid="report-scorecard">
            <h3>The scorecard</h3>
            <p>
              Every criterion the panel scored, out of ten, with the share of the task's
              points it carried.
            </p>
            <ul class="sheet">
              {#each scorecard as row (row.key)}
                <li data-testid="report-scorecard-row">
                  <div class="sheet-head">
                    <b>{row.title}</b>
                    <span class="muted">
                      {Math.round(row.weight * 100)}% of the task · scored by {judgeLabel(row.judge)}
                    </span>
                    <b class="sheet-score {scoreTone(row.score)}">
                      {row.score === null ? "—" : row.score.toFixed(1)}<span class="muted">/10</span>
                    </b>
                  </div>
                  <!-- The score as a bar as well as a number: eleven criteria
                       read as a shape, and the weak ones have to be findable
                       without reading every line. -->
                  <div class="meter" aria-hidden="true">
                    <span
                      class={scoreTone(row.score)}
                      style="width: {Math.max(0, Math.min(10, row.score ?? 0)) * 10}%"
                    ></span>
                  </div>
                  <MarkdownContent
                    value={markdownSafe(row.rationale)}
                    class="mt-2 text-[15px]"
                  />
                </li>
              {/each}
            </ul>
          </section>
        {/if}

        {#if (doc.improve ?? []).length > 0}
          <section data-testid="report-improve">
            <h3>How to improve results</h3>
            <ol class="doc-list">
              {#each doc.improve ?? [] as step, i (i)}
                <li>{step}</li>
              {/each}
            </ol>
          </section>
        {/if}
      {:else}
        <!-- The model answered in prose instead of the document: render what
             it wrote rather than dropping the report. -->
        <MarkdownContent value={report.markdown} class="text-[15px]" />
      {/if}
    </article>
  {:else if judgesSettling}
    <div class="doc text-center" data-testid="session-report-pending">
      <h3 class="!mt-0">Writing your report…</h3>
      <p>It goes last, once every judge has finished — so it can tell you what they said.</p>
    </div>
  {:else if sessionFinished}
    <!-- Why there is none, we cannot tell from here: the project may not run a
         reporter, or the run may have failed. The old copy asserted the first
         and was wrong for a whole class of sessions. -->
    <div class="doc text-center">
      <p>No report was written for this session.</p>
    </div>
  {:else}
    <div class="doc text-center">
      <p>Your report is written when the session ends.</p>
    </div>
  {/if}

  {#if lightboxShot}
    <ImageLightbox
      src={lightboxShot.src}
      alt={lightboxShot.label}
      counter={galleryShots.length > 1
        ? `${(lightboxIndex ?? 0) + 1} / ${galleryShots.length}`
        : null}
      onprev={galleryShots.length > 1 ? () => stepLightbox(-1) : null}
      onnext={galleryShots.length > 1 ? () => stepLightbox(1) : null}
      onclose={() => (lightboxIndex = null)}
    />
  {/if}
</section>

<style>
  /* The documentation's article block, at the width this column allows.
     Values are the ones /documentation renders, so the two pages read as one
     product rather than two. */
  .doc {
    background: white;
    border-radius: 8px;
    padding: 40px 56px 48px;
  }

  @media (max-width: 767px) {
    .doc {
      padding: 24px 20px 32px;
    }
  }

  .doc h2 {
    font-size: 40px;
    font-weight: 700;
    color: #363636;
    line-height: 1.2;
    margin-bottom: 24px;
  }

  .doc h3 {
    font-size: 26px;
    font-weight: 700;
    color: #363636;
    line-height: 1.23;
    margin-top: 40px;
    margin-bottom: 16px;
  }

  @media (max-width: 767px) {
    .doc h2 {
      font-size: 28px;
    }

    .doc h3 {
      font-size: 22px;
    }
  }

  .doc p {
    font-size: 15px;
    color: #363636;
    line-height: 1.7;
    margin-top: 16px;
    margin-bottom: 16px;
  }

  /* The breadcrumb strip, reused as the byline: who wrote this, and when. */
  .breadcrumbs {
    display: flex;
    flex-wrap: wrap;
    font-size: 16px;
    font-weight: 700;
    line-height: 1.5;
    color: #9ea7b6;
    margin-bottom: 24px;
    list-style: none;
    padding: 0;
  }

  .breadcrumbs li {
    position: relative;
    display: block;
  }

  .breadcrumbs li:not(:last-child) {
    padding-right: 32px;
  }

  .breadcrumbs li:not(:last-child)::before {
    position: absolute;
    content: "";
    top: 5px;
    right: 12px;
    width: 12px;
    height: 12px;
    background-size: contain;
    background-repeat: no-repeat;
    background-position: center;
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='306' height='306' viewBox='0 0 306 306'%3E%3Cpath fill='%239ea7b6' d='M94.35 0l-35.7 35.7L175.95 153 58.65 270.3l35.7 35.7 153-153z'/%3E%3C/svg%3E");
  }

  /* Bullets and counters, both from the documentation. */
  .doc-list {
    margin-top: 16px;
    margin-bottom: 24px;
    padding: 0;
    list-style: none;
  }

  ol.doc-list {
    counter-reset: list;
  }

  .doc-list li {
    position: relative;
    display: block;
    padding-left: 36px;
    font-size: 15px;
    color: #363636;
    line-height: 1.7;
  }

  ul.doc-list li::before {
    position: absolute;
    content: "";
    top: 9px;
    left: 13px;
    width: 4px;
    height: 4px;
    border-radius: 50%;
    background-color: #0269fb;
  }

  ol.doc-list li::before {
    position: absolute;
    counter-increment: list;
    content: counter(list) ".";
    top: 0;
    left: 13px;
    color: #363636;
    font-weight: 700;
  }

  /* The documentation's aside, carrying one rough patch each. */
  .doc blockquote {
    background: #dce9fc;
    color: #36547f;
    line-height: 1.5;
    padding: 16px 32px;
    margin-top: 16px;
    margin-bottom: 16px;
    border-radius: 8px;
    display: block;
  }

  .doc blockquote b {
    font-size: 15px;
    font-weight: 700;
  }

  .doc blockquote p {
    color: #36547f;
    font-style: italic;
    font-weight: 500;
    margin: 4px 0 0;
  }

  /* ── The panel ────────────────────────────────────────────────────────
     One block per task, one card per verdict inside it. Cards rather than
     rows: a verdict is a paragraph with a number attached, and three columns
     of prose was unreadable the moment a judge wrote more than a line. */
  .panel-task {
    margin-top: 24px;
  }

  .panel-task-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    padding-bottom: 8px;
    border-bottom: 1px solid #dce9fc;
    font-size: 15px;
    color: #363636;
  }

  .verdict {
    border: 1px solid #dce9fc;
    border-radius: 8px;
    padding: 16px 20px;
    margin-top: 12px;
  }

  .verdict header {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 10px;
  }

  .who-avatar {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    object-fit: cover;
    flex-shrink: 0;
  }

  .who-avatar.fallback {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: #eef4fd;
    font-size: 14px;
  }

  .verdict header .who {
    font-size: 16px;
    font-weight: 700;
    color: #363636;
  }

  .pts {
    margin-left: auto;
    font-size: 14px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .panel-task-head .pts {
    margin-left: 0;
  }

  .pos {
    color: #15803d;
  }

  .neg {
    color: #dc2626;
  }

  .warn {
    color: #b45309;
  }

  .muted {
    color: #9ea7b6;
  }

  /* The reporter's two lines, as a definition list: the label carries the
     question, so neither sentence has to repeat it. */
  .brief {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px 16px;
    margin: 12px 0 0;
  }

  .brief dt {
    font-size: 12px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: #9ea7b6;
    padding-top: 3px;
  }

  .brief dd {
    margin: 0;
    font-size: 15px;
    line-height: 1.6;
    color: #363636;
  }

  @media (max-width: 767px) {
    .brief {
      grid-template-columns: 1fr;
      gap: 2px;
    }

    .brief dd {
      margin-bottom: 8px;
    }
  }

  /* The judge's own words, set apart from the summary above them. */
  .verdict-text {
    margin-top: 12px;
    padding-top: 12px;
    border-top: 1px solid #eef4fd;
    font-size: 15px;
    line-height: 1.7;
    color: #363636;
  }

  .doc p.muted {
    color: #9ea7b6;
    font-size: 14px;
  }

  /* The criteria sheet. A table put four columns of prose into a 300px
     "why" — rows that are full-width blocks let the rationale read. */
  .sheet {
    list-style: none;
    margin: 24px 0;
    padding: 0;
  }

  .sheet li {
    padding: 16px 0;
    border-top: 1px solid #eef4fd;
  }

  .sheet li:first-child {
    border-top: none;
  }

  .sheet-head {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 4px 12px;
    font-size: 15px;
    color: #363636;
  }

  .sheet-head .muted {
    font-size: 13px;
  }

  .sheet-score {
    margin-left: auto;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .sheet-score .muted {
    font-weight: 400;
    font-size: 13px;
  }

  .meter {
    height: 4px;
    border-radius: 2px;
    background: #eef4fd;
    margin-top: 8px;
    overflow: hidden;
  }

  .meter span {
    display: block;
    height: 100%;
    border-radius: 2px;
    background: currentColor;
  }


  /* The panel, as the documentation draws a table. Scrolls rather than
     squeezing three columns onto a phone. */
  .table-scroll {
    overflow-x: auto;
    margin-top: 24px;
    margin-bottom: 24px;
  }

  .doc table {
    width: 100%;
    min-width: 560px;
    border-collapse: separate;
    border-spacing: 0;
    font-size: 15px;
    line-height: 1.5;
    border: 1px solid #dce9fc;
    border-radius: 8px;
  }

  .doc th {
    background: #eef4fd;
    color: #36547f;
    font-size: 14px;
    font-weight: 600;
    text-align: left;
    padding: 12px 20px;
  }

  .doc th:first-child {
    border-top-left-radius: 7px;
  }

  .doc th:last-child {
    border-top-right-radius: 7px;
  }

  .doc td {
    padding: 12px 20px;
    color: #363636;
    border-top: 1px solid #eef4fd;
    vertical-align: top;
  }

  .doc td.who {
    font-weight: 700;
    white-space: nowrap;
  }

  /* Captures as a brick wall: columns of a fixed minimum width, each capture
     at its own aspect ratio. A row of equal-height thumbnails cropped every
     one of them to the same letterbox — a phone screenshot and a dashboard
     are not the same shape, and the shape is half of what a screenshot says.
     CSS columns rather than grid: masonry is still not shipped everywhere,
     and reading order down a column is fine for a gallery. */
  .shots {
    columns: 300px;
    column-gap: 16px;
    margin-top: 24px;
  }

  .shots figure {
    break-inside: avoid;
    margin: 0 0 16px;
  }

  /* Bricks, but bounded: a phone screenshot is twice as tall as it is wide,
     and left to its ratio it ran to half a screen and buried everything
     under it. Cropped from the top, where a screenshot keeps its subject —
     the whole image is one click away. */
  .shots img,
  .shots video {
    width: 100%;
    height: auto;
    max-height: 320px;
    object-fit: cover;
    object-position: top;
    border-radius: 8px;
    display: block;
  }

  /* A button, so the keyboard reaches it — drawn as the bare image it was. */
  .shots .thumb {
    display: block;
    width: 100%;
    padding: 0;
    border: 1px solid #dce9fc;
    border-radius: 8px;
    background: none;
    overflow: hidden;
    cursor: zoom-in;
    transition: box-shadow 150ms ease;
  }

  .shots .thumb:hover {
    box-shadow: 0 4px 12px rgb(54 84 127 / 18%);
  }

  .shots .thumb img {
    transition: transform 150ms ease;
  }

  .shots .thumb:hover img {
    transform: scale(1.05);
  }

  @media (prefers-reduced-motion: reduce) {
    .shots .thumb img,
    .shots .thumb:hover img {
      transition: none;
      transform: none;
    }
  }

  .shots figcaption {
    margin-top: 6px;
    font-size: 13px;
    color: #9ea7b6;
  }

  .shots .file {
    display: block;
    border: 1px solid #dce9fc;
    border-radius: 8px;
    padding: 12px 20px;
    font-size: 15px;
    color: #0269fb;
    text-decoration: underline;
  }
</style>
