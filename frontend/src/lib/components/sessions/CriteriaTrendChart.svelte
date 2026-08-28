<script lang="ts">
  import type { PlayerTaskEvaluation, PlayerTaskSummaryEntry } from '$lib/types/arena';

  // How the panel's per-criterion scores move task by task.
  //
  // Form: small multiples, not one multi-line chart. A project carries ~11
  // criteria; eleven lines on one plot is spaghetti, and eleven categorical
  // hues is past the point where anyone (colorblind or not) can tell them
  // apart. One facet per criterion, each a single series, all sharing the
  // 0–10 scale — so comparing two facets is honest — and each labelled by
  // its own title, so identity never rides on color.
  type Props = {
    /** Tasks in display order; only revealed tasks should be passed in. */
    tasks: PlayerTaskSummaryEntry[];
    evaluations: PlayerTaskEvaluation[];
  };
  let { tasks, evaluations }: Props = $props();

  type Point = { ordinal: number; score: number | null };
  type Facet = { key: string; title: string; points: Point[]; last: number | null; delta: number | null };

  const SCALE_MAX = 10;

  const evalByTask = $derived(new Map(evaluations.map((e) => [e.task_id, e])));

  /** Tasks that carry a criteria sheet, in ordinal order. */
  const scoredTasks = $derived(
    [...tasks]
      .sort((a, b) => a.ordinal - b.ordinal)
      .filter((t) => (evalByTask.get(t.task_id)?.criteria?.length ?? 0) > 0),
  );

  /** One facet per criterion; a task where a criterion went unscored is a
   *  gap, never an interpolated value. */
  const facets = $derived.by<Facet[]>(() => {
    const titles = new Map<string, string>();
    const order: string[] = [];
    for (const t of scoredTasks) {
      for (const c of evalByTask.get(t.task_id)?.criteria ?? []) {
        if (!titles.has(c.key)) {
          titles.set(c.key, c.title || c.key);
          order.push(c.key);
        }
      }
    }
    return order
      .map((key) => {
        const points: Point[] = scoredTasks.map((t) => {
          const c = evalByTask.get(t.task_id)?.criteria?.find((x) => x.key === key);
          const graded = (c?.scores ?? []).filter((s) => typeof s.score === 'number');
          const score = graded.length
            ? graded.reduce((sum, s) => sum + (s.score as number), 0) / graded.length
            : null;
          return { ordinal: t.ordinal, score };
        });
        const scored = points.filter((p) => p.score !== null);
        const last = scored.length ? (scored[scored.length - 1].score as number) : null;
        const delta =
          scored.length >= 2 ? (last as number) - (scored[0].score as number) : null;
        return { key, title: titles.get(key) ?? key, points, last, delta };
      })
      .filter((f) => f.points.some((p) => p.score !== null));
  });

  /** A trend needs two points to be a trend. */
  const hasTrend = $derived(scoredTasks.length >= 2 && facets.length > 0);

  // Table first: it answers "what did each criterion score" outright, and
  // the chart is the follow-up question.
  let view = $state<'table' | 'chart'>('table');

  // Folded by default — the tab's job is the verdicts; this is context a
  // reader opens when they want the arc.
  let open = $state(false);

  // ── Geometry ──────────────────────────────────────────────────────────
  // The viewBox scales uniformly (no stretch), so dots stay round; the line
  // and the axis declare non-scaling strokes to keep their exact weights.
  const VB_W = 240;
  const VB_H = 64;
  const PAD_Y = 8;

  /** One domain for every facet — comparing two facets must not lie — but
   *  tightened to the data: criteria live in a narrow band near the top of
   *  0–10, and plotting the empty bottom half flattens every trend into a
   *  straight line. The band in force is named in the subtitle. */
  const domain = $derived.by(() => {
    const all = facets.flatMap((f) => f.points.map((p) => p.score)).filter((s): s is number => s !== null);
    if (all.length === 0) return { lo: 0, hi: SCALE_MAX };
    const lo = Math.max(0, Math.floor(Math.min(...all)) - 1);
    const hi = Math.min(SCALE_MAX, Math.ceil(Math.max(...all)) + 1);
    // A flat session (every score equal) still needs a non-zero span.
    return hi - lo < 1 ? { lo: Math.max(0, lo - 1), hi: Math.min(SCALE_MAX, lo + 1) } : { lo, hi };
  });

  function x(i: number, n: number): number {
    if (n <= 1) return VB_W / 2;
    // Inset so the end dot is not clipped by the viewBox edge.
    const pad = 6;
    return pad + (i / (n - 1)) * (VB_W - pad * 2);
  }

  function y(score: number): number {
    const { lo, hi } = domain;
    const t = Math.max(0, Math.min(1, (score - lo) / (hi - lo)));
    return VB_H - PAD_Y - t * (VB_H - PAD_Y * 2);
  }

  /** Contiguous runs of scored points — a gap breaks the line rather than
   *  drawing through a task the judges never scored. */
  function segments(points: Point[]): string[] {
    const out: string[] = [];
    let run: string[] = [];
    points.forEach((p, i) => {
      if (p.score === null) {
        if (run.length > 1) out.push(run.join(' '));
        run = [];
        return;
      }
      run.push(`${run.length === 0 ? 'M' : 'L'} ${x(i, points.length)} ${y(p.score)}`);
    });
    if (run.length > 1) out.push(run.join(' '));
    return out;
  }

  /** A lone scored point has no line to draw — mark it so it is not invisible. */
  function loneDots(points: Point[]): { cx: number; cy: number }[] {
    return points
      .map((p, i) => ({ p, i }))
      .filter(({ p, i }) => {
        if (p.score === null) return false;
        const prev = points[i - 1]?.score ?? null;
        const next = points[i + 1]?.score ?? null;
        return prev === null && next === null;
      })
      .map(({ p, i }) => ({ cx: x(i, points.length), cy: y(p.score as number) }));
  }

  function lastScoredIndex(points: Point[]): number {
    for (let i = points.length - 1; i >= 0; i--) if (points[i].score !== null) return i;
    return -1;
  }

  function fmt(score: number | null): string {
    return score === null ? '—' : score.toFixed(1);
  }

  function fmtDelta(d: number): string {
    return `${d > 0 ? '+' : d < 0 ? '−' : '±'}${Math.abs(d).toFixed(1)}`;
  }

  // Hover: which (facet, point) the reader is on, and where to put the label.
  let hover = $state<{ key: string; index: number } | null>(null);
</script>

{#if hasTrend}
  <section class="mb-6 rounded-2xl bg-white p-5 shadow-sm" data-testid="criteria-trend">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <button
        type="button"
        class="flex min-w-0 items-center gap-2 text-left"
        aria-expanded={open}
        onclick={() => (open = !open)}
        data-testid="criteria-toggle"
      >
        <svg
          width="10"
          height="10"
          viewBox="0 0 10 10"
          class="shrink-0 text-brand-muted transition-transform {open ? 'rotate-90' : ''}"
          aria-hidden="true"
        >
          <path d="M3 1l4 4-4 4" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
        <span class="min-w-0">
          <span class="font-heading text-[15px] font-bold text-brand-text">Criteria across tasks</span>
          <span class="ml-2 text-[12px] text-brand-muted">
            {facets.length}
            {facets.length === 1 ? 'criterion' : 'criteria'} · {scoredTasks.length} tasks
          </span>
        </span>
      </button>
      {#if open}
        <div class="inline-flex shrink-0 rounded-[8px] bg-brand-light-blue p-1" role="group" aria-label="Criteria view">
          {#each [{ id: 'table' as const, label: 'Table' }, { id: 'chart' as const, label: 'Chart' }] as opt (opt.id)}
            <button
              type="button"
              onclick={() => (view = opt.id)}
              aria-pressed={view === opt.id}
              data-testid="criteria-view-{opt.id}"
              class="rounded-[6px] px-2.5 py-0.5 text-[12px] font-semibold transition-colors
                     {view === opt.id ? 'bg-white text-brand-blue shadow-sm' : 'text-brand-muted hover:text-brand-text'}"
            >
              {opt.label}
            </button>
          {/each}
        </div>
      {/if}
    </div>

    {#if open}
      <p class="mt-1 text-[12px] text-brand-muted">
        The panel's average score per criterion, task by task.{#if view === 'chart'}
          Every facet shares the {domain.lo}–{domain.hi} band of the 0–{SCALE_MAX} scale.
        {/if}
      </p>

    {#if view === 'chart'}
      <div class="mt-4 grid grid-cols-1 gap-x-5 gap-y-4 sm:grid-cols-2 lg:grid-cols-3">
        {#each facets as f (f.key)}
          {@const lastIdx = lastScoredIndex(f.points)}
          <figure class="min-w-0" data-testid="criteria-facet-{f.key}">
            <figcaption class="mb-1 flex items-baseline justify-between gap-2">
              <span class="min-w-0 truncate text-[12px] font-semibold text-brand-text" title={f.title}>
                {f.title}
              </span>
              <span class="flex shrink-0 items-baseline gap-1.5">
                <!-- Selective direct label: the latest value, not every point. -->
                <span class="text-[13px] font-bold tabular-nums text-brand-text">{fmt(f.last)}</span>
                {#if f.delta !== null && Math.abs(f.delta) >= 0.05}
                  <!-- Direction is spelled by the glyph and the sign, so the
                       red/green pair is never the only channel. -->
                  <span
                    class="text-[11px] font-bold tabular-nums {f.delta > 0 ? 'text-green-600' : 'text-brand-red'}"
                    data-testid="criteria-delta-{f.key}"
                  >
                    {f.delta > 0 ? '▲' : '▼'}{fmtDelta(f.delta)}
                  </span>
                {/if}
              </span>
            </figcaption>

            <div class="relative">
              <svg
                viewBox="0 0 {VB_W} {VB_H}"
                style="aspect-ratio: {VB_W} / {VB_H}"
                class="w-full"
                role="img"
                aria-label="{f.title}: {f.points
                  .filter((p) => p.score !== null)
                  .map((p) => `task ${p.ordinal} ${fmt(p.score)}`)
                  .join(', ')}"
              >
                <!-- Recessive baseline: hairline, solid, one step off surface. -->
                <line
                  x1="0"
                  y1={VB_H - PAD_Y}
                  x2={VB_W}
                  y2={VB_H - PAD_Y}
                  stroke="#e7eaee"
                  stroke-width="1"
                  vector-effect="non-scaling-stroke"
                />
                {#each segments(f.points) as d (d)}
                  <path
                    {d}
                    fill="none"
                    stroke="#0269fb"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    vector-effect="non-scaling-stroke"
                  />
                {/each}
                <!-- Zero-length round-capped strokes: perfect circles that the
                     viewBox stretch cannot turn into ellipses. -->
                {#each loneDots(f.points) as dot (`${dot.cx}-${dot.cy}`)}
                  <circle cx={dot.cx} cy={dot.cy} r="3.5" fill="#0269fb" />
                {/each}
                {#if lastIdx >= 0}
                  {@const cx = x(lastIdx, f.points.length)}
                  {@const cy = y(f.points[lastIdx].score as number)}
                  <!-- 2px surface ring so the end dot stays legible over the line. -->
                  <circle {cx} {cy} r="6" fill="#ffffff" />
                  <circle {cx} {cy} r="4" fill="#0269fb" />
                {/if}
              </svg>

              <!-- Hover layer: one hit band per task, wider than the mark. -->
              <div class="absolute inset-0 flex">
                {#each f.points as p, i (p.ordinal)}
                  <button
                    type="button"
                    class="h-full flex-1 cursor-default bg-transparent"
                    aria-label="Task {p.ordinal}: {fmt(p.score)}"
                    onmouseenter={() => (hover = { key: f.key, index: i })}
                    onmouseleave={() => (hover = null)}
                    onfocus={() => (hover = { key: f.key, index: i })}
                    onblur={() => (hover = null)}
                  ></button>
                {/each}
              </div>

              {#if hover?.key === f.key}
                {@const p = f.points[hover.index]}
                <div
                  style="left: {((hover.index + 0.5) / f.points.length) * 100}%"
                  class="pointer-events-none absolute -top-2 z-10 -translate-x-1/2 whitespace-nowrap rounded-md bg-brand-text px-2 py-1 text-[11px] font-medium text-white shadow-md"
                  role="status"
                  data-testid="criteria-tooltip"
                >
                  Task #{p.ordinal} · {fmt(p.score)}{p.score === null ? ' (not scored)' : ''}
                </div>
              {/if}
            </div>

            <!-- Only the ends carry an axis label; the tooltip has the rest. -->
            <div class="mt-0.5 flex justify-between text-[10px] tabular-nums text-brand-muted">
              <span>#{f.points[0]?.ordinal}</span>
              {#if f.points.length > 1}
                <span>#{f.points[f.points.length - 1]?.ordinal}</span>
              {/if}
            </div>
          </figure>
        {/each}
      </div>
    {:else}
      <!-- The table is the accessible twin, and the honest home for eleven
           series: every number, no color needed. -->
      <div class="mt-4 overflow-x-auto">
        <table class="w-full min-w-[420px] border-collapse text-[12px]" data-testid="criteria-table">
          <thead>
            <tr class="border-b border-brand-border text-brand-muted">
              <th scope="col" class="py-1.5 pr-3 text-left font-semibold">Criterion</th>
              {#each scoredTasks as t (t.task_id)}
                <th scope="col" class="px-2 py-1.5 text-right font-semibold tabular-nums">#{t.ordinal}</th>
              {/each}
              <th scope="col" class="py-1.5 pl-2 text-right font-semibold">Δ</th>
            </tr>
          </thead>
          <tbody>
            {#each facets as f (f.key)}
              <tr class="border-b border-brand-border/50">
                <th scope="row" class="py-1.5 pr-3 text-left font-medium text-brand-text">{f.title}</th>
                {#each f.points as p (p.ordinal)}
                  <td class="px-2 py-1.5 text-right tabular-nums {p.score === null ? 'text-brand-muted' : 'text-brand-text'}">
                    {fmt(p.score)}
                  </td>
                {/each}
                <td class="py-1.5 pl-2 text-right font-semibold tabular-nums">
                  {#if f.delta === null}
                    <span class="text-brand-muted">—</span>
                  {:else}
                    <span class={f.delta > 0 ? 'text-green-600' : f.delta < 0 ? 'text-brand-red' : 'text-brand-muted'}>
                      {f.delta > 0 ? '▲' : f.delta < 0 ? '▼' : ''}{fmtDelta(f.delta)}
                    </span>
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
    {/if}
  </section>
{/if}
