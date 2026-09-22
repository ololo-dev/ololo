<script lang="ts">
  import { browser } from "$app/environment";
  import { onDestroy } from "svelte";
  import type {
    HealthCheckpointView,
    LeaderboardEntry,
    MemberInfo,
    ScoreHistoryPoint,
    SessionHealthPayload,
  } from "$lib/types/arena";
  import { LEVEL_COLORS, formatScore, gradeOf, seriesColor } from "$lib/session-health";
  import {
    buildChartData,
    describePoints,
    taskAt,
    type ChartMember,
    type PointMeta,
  } from "$lib/score-chart-data";

  type Phase = "lobby" | "active" | "paused" | "finished";

  let {
    phase,
    leaderboard,
    reportMembers,
    scoreHistory,
    health = null,
    revealUntil = null,
  }: {
    phase: Phase;
    leaderboard: LeaderboardEntry[];
    reportMembers: MemberInfo[];
    scoreHistory: ScoreHistoryPoint[];
    /** Every participant's code-health history. With it, every probe sits
     * on the participant's line as a marker in its health colour; without
     * it the chart is the plain points line. */
    health?: SessionHealthPayload | null;
    /** Replay playhead (elapsed seconds): points past it are hidden while the
     * full time axis stays fixed, so the line draws in. `null` shows all. */
    revealUntil?: number | null;
  } = $props();

  let chartEl: HTMLDivElement | undefined = $state(undefined);
  // $state.raw: tracks the reference reactively but doesn't proxy the uPlot instance itself
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let uplotInstance = $state.raw<any>(null);
  let chartMembers: ChartMember[] = [];
  let chartBuilding = false; // reentry guard — prevents concurrent async buildChart calls

  // What the draw hooks read: the health payload behind the current data
  // and, per participant and data index, what that point stands for.
  let hookHealth: SessionHealthPayload | null = null;
  let hookMeta: Map<string, PointMeta> = new Map();
  let hookCut: number | null = null;
  let hookShowHealthLabels = true;
  let hookShowPointLabels = true;

  /** Whether the labels are drawn — the checks' grade pills and the points
   * pills on judge verdicts — per-viewer conveniences remembered in this
   * browser; the markers and the tooltip stay either way. */
  const HEALTH_LABELS_KEY = "ololo.chart.healthLabels";
  const POINT_LABELS_KEY = "ololo.chart.pointLabels";
  function readLabelSetting(key: string): boolean {
    try {
      return localStorage.getItem(key) !== "off";
    } catch {
      // Storage unavailable (private window, blocked site data): default on.
      return true;
    }
  }
  function writeLabelSetting(key: string, on: boolean) {
    try {
      localStorage.setItem(key, on ? "on" : "off");
    } catch {
      // Ignore: the choice just does not survive this page.
    }
  }
  let showHealthLabels = $state(browser ? readLabelSetting(HEALTH_LABELS_KEY) : true);
  let showPointLabels = $state(browser ? readLabelSetting(POINT_LABELS_KEY) : true);
  function toggleHealthLabels() {
    showHealthLabels = !showHealthLabels;
    writeLabelSetting(HEALTH_LABELS_KEY, showHealthLabels);
  }
  function togglePointLabels() {
    showPointLabels = !showPointLabels;
    writeLabelSetting(POINT_LABELS_KEY, showPointLabels);
  }
  $effect(() => {
    hookShowHealthLabels = showHealthLabels;
    hookShowPointLabels = showPointLabels;
    uplotInstance?.redraw();
  });

  /** Tooltip for the point under the cursor. */
  let tip = $state<{ x: number; y: number; lines: { label: string; value: string; tone?: string }[] } | null>(
    null,
  );


  // Format elapsed seconds as H:MM:SS (or M:SS) — values are seconds since
  // session start, NOT unix timestamps, so we disable uPlot time mode.
  function fmtElapsed(v: number): string {
    const t = Math.max(0, Math.round(v));
    const h = Math.floor(t / 3600);
    const m = Math.floor((t % 3600) / 60);
    const s = t % 60;
    const mm = String(m).padStart(2, "0");
    const ss = String(s).padStart(2, "0");
    return h > 0 ? `${h}:${mm}:${ss}` : `${m}:${ss}`;
  }

  async function buildChart(members: ChartMember[]) {
    if (!chartEl || chartBuilding) return;
    chartBuilding = true;

    uplotInstance?.destroy();
    uplotInstance = null;

    // CSS: fire-and-forget so a load failure never aborts chart creation
    import("uplot/dist/uPlot.min.css").catch(() => {});

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    let uPlot: any;
    try {
      const mod = await import("uplot");
      uPlot = mod.default;
    } catch (err) {
      console.error("[chart] uPlot load failed:", err);
      chartBuilding = false;
      return;
    }

    // Guard: component may have unmounted while the import was in flight
    if (!chartEl) {
      chartBuilding = false;
      return;
    }

    // Defer to next animation frame so the container has been laid out and
    // clientWidth reflects the actual rendered width (not 0). The component
    // may unmount before the frame fires (bind:this clears chartEl), so read
    // it defensively — a throw here escapes every await and surfaces as an
    // uncaught exception; the guard below then bails out of the build.
    const width = await new Promise<number>((resolve) => {
      requestAnimationFrame(() => resolve(chartEl?.clientWidth || 600));
    });

    if (!chartEl) {
      chartBuilding = false;
      return;
    }

    const series = [
      {
        label: "Time",
        // Disable time mode so uPlot does not format x as a unix timestamp
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        value: (_u: any, v: number) => fmtElapsed(v),
      },
      // One line per participant. The markers (every probe in its health
      // colour, every scored change) are drawn by the hook below, not by
      // uPlot, so their shape can say what they are.
      ...members.map((p, i) => ({
        label: p.display_name,
        stroke: seriesColor(i),
        width: 2,
        spanGaps: true,
        points: { show: false },
      })),
    ];

    const chartData: number[][] = [[], ...series.slice(1).map(() => [])];

    const scales: Record<string, unknown> = {
      // x values are elapsed seconds, not unix timestamps — disable
      // time mode so tooltips do not render "1970-01-01 3:44am".
      x: { time: false },
      y: {
        // Always include 0 so negative scores don't fill the whole axis
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        range: (_u: any, dataMin: number, dataMax: number) => {
          const lo = Math.min(0, dataMin ?? 0);
          const hi = Math.max(0, dataMax ?? 0);
          const span = Math.max(hi - lo, 10);
          const pad = span * 0.15;
          return [lo - pad, hi + pad];
        },
      },
    };

    const axes = [
      {
        label: "Time (s)",
        labelSize: 18,
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        values: (_u: any, vals: number[]) =>
          vals.map((v: number) => {
            const t = Math.round(v);
            const m = Math.floor(t / 60);
            const s = t % 60;
            return m > 0 ? `${m}:${String(s).padStart(2, "0")}` : `${s}s`;
          }),
        stroke: "#8fb4ec",
        font: "11px sans-serif",
        labelFont: "bold 11px sans-serif",
        grid: { stroke: "#f0f4fa", width: 1 },
        ticks: { stroke: "#e0e8f0", width: 1 },
      },
      {
        label: "Points",
        labelSize: 32,
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        values: (_u: any, vals: number[]) => vals.map((v) => String(Math.round(v))),
        stroke: "#8fb4ec",
        font: "11px sans-serif",
        labelFont: "bold 11px sans-serif",
        grid: { stroke: "#f0f4fa", width: 1 },
        ticks: { stroke: "#e0e8f0", width: 1 },
        space: 40,
      },
    ];

    try {
      uplotInstance = new uPlot(
        {
          width,
          height: 240,
          series,
          scales,
          hooks: {
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
            draw: [(u: any) => {
              drawZeroLine(u);
              drawTaskSeparators(u, members);
              drawPoints(u, members);
            }],
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
            setCursor: [(u: any) => updateTooltip(u, members)],
          },
          axes,
        },
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        chartData as any,
        chartEl,
      );
      chartMembers = members;
    } catch (err) {
      console.error("[chart] uPlot init failed:", err);
    } finally {
      chartBuilding = false;
    }
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  function drawZeroLine(u: any) {
    const y0 = u.valToPos(0, "y", true);
    const { left, top, width: w, height: h } = u.bbox;
    if (y0 < top || y0 > top + h) return;
    const ctx: CanvasRenderingContext2D = u.ctx;
    ctx.save();
    ctx.strokeStyle = "rgba(100, 120, 160, 0.25)";
    ctx.lineWidth = 1;
    ctx.setLineDash([4, 4]);
    ctx.beginPath();
    ctx.moveTo(left, y0);
    ctx.lineTo(left + w, y0);
    ctx.stroke();
    ctx.restore();
  }

  /** Vertical dotted separators where each participant's tasks start,
   * tinted with the participant's colour. The tooltip names the task. */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  function drawTaskSeparators(u: any, members: ChartMember[]) {
    if (!hookHealth) return;
    const { left, width: w, top, height: h } = u.bbox;
    const ctx: CanvasRenderingContext2D = u.ctx;
    ctx.save();
    members.forEach((m, i) => {
      const ranges = hookHealth?.players[m.player_id]?.task_ranges ?? [];
      ctx.strokeStyle = seriesColor(i);
      ctx.globalAlpha = 0.55;
      ctx.lineWidth = 1;
      ctx.setLineDash([2, 3]);
      for (const r of ranges) {
        const t = r.start_t;
        if (t == null || (hookCut != null && t > hookCut)) continue;
        // The first task starts at t = 0, on the axis itself: keep a marker
        // that lands a sub-pixel outside the plot instead of dropping it.
        const x = Math.min(Math.max(u.valToPos(t, "x", true), left), left + w);
        if (x < left - 1 || x > left + w + 1) continue;
        ctx.beginPath();
        ctx.moveTo(x, top);
        ctx.lineTo(x, top + h);
        ctx.stroke();
      }
    });
    ctx.restore();
  }

  /** What a label on the chart says: a check's grade and score, `[B] 75.4`,
   * or a plain figure such as the points a judge awarded. */
  type Pill =
    | { kind: "health"; grade: string | null; score: number | null; color: string }
    | { kind: "plain"; text: string; color: string };

  const PILL_H = 14;
  const PILL_PAD = 4;
  const GRADE_BOX = 12;

  function pillWidth(ctx: CanvasRenderingContext2D, pill: Pill, dpr: number): number {
    if (pill.kind === "plain") return ctx.measureText(pill.text).width + PILL_PAD * 2 * dpr;
    return (GRADE_BOX + 3 + 2) * dpr + ctx.measureText(formatScore(pill.score)).width;
  }

  function roundedRect(ctx: CanvasRenderingContext2D, left: number, top: number, w: number, h: number, r: number) {
    ctx.beginPath();
    if (typeof ctx.roundRect === "function") {
      ctx.roundRect(left, top, w, h, r);
    } else {
      ctx.moveTo(left + r, top);
      ctx.arcTo(left + w, top, left + w, top + h, r);
      ctx.arcTo(left + w, top + h, left, top + h, r);
      ctx.arcTo(left, top + h, left, top, r);
      ctx.arcTo(left, top, left + w, top, r);
      ctx.closePath();
    }
  }

  /** Draw a pill centred on `x`, its edge at `yEdge` (above or below the
   * marker). A health pill is the grade in a filled box and the score
   * beside it, haloed in white so it reads over the line — the same badge
   * as the players' list, no frame. A plain pill is solid. Returns its
   * horizontal extent. */
  function drawPill(
    ctx: CanvasRenderingContext2D,
    x: number,
    yEdge: number,
    pill: Pill,
    dpr: number,
    above: boolean,
  ): [number, number] {
    const h = PILL_H * dpr;
    const w = pillWidth(ctx, pill, dpr);
    const left = x - w / 2;
    const top = above ? yEdge - h : yEdge;
    const midY = top + h / 2 + 0.5 * dpr;
    ctx.textBaseline = "middle";
    if (pill.kind === "plain") {
      roundedRect(ctx, left, top, w, h, h / 2);
      ctx.fillStyle = pill.color;
      ctx.fill();
      ctx.fillStyle = "#ffffff";
      ctx.textAlign = "center";
      ctx.fillText(pill.text, x, midY);
      return [left, left + w];
    }
    const boxLeft = left;
    const box = GRADE_BOX * dpr;
    // A white halo under the box and the score keeps them legible where
    // the line or the grid runs behind them.
    ctx.lineJoin = "round";
    ctx.lineWidth = 3 * dpr;
    ctx.strokeStyle = "rgba(255,255,255,0.9)";
    roundedRect(ctx, boxLeft, top + (h - box) / 2, box, box, 3 * dpr);
    ctx.stroke();
    ctx.fillStyle = pill.color;
    ctx.fill();
    ctx.fillStyle = "#ffffff";
    ctx.textAlign = "center";
    ctx.fillText(pill.grade ?? "–", boxLeft + box / 2, midY);
    const score = formatScore(pill.score);
    const scoreX = boxLeft + box + 3 * dpr;
    ctx.textAlign = "left";
    ctx.strokeText(score, scoreX, midY);
    ctx.fillStyle = pill.color;
    ctx.fillText(score, scoreX, midY);
    return [left, left + w];
  }

  /** `B 75.4` for the tooltip, `—` when there was nothing to score. */
  function healthLabel(cp: HealthCheckpointView): string {
    if (cp.score == null) return "—";
    const grade = gradeOf(cp);
    return `${grade ? `${grade} ` : ""}${formatScore(cp.score)}`;
  }

  function judgeDelta(meta: PointMeta): number | null {
    const verdicts = meta.changes.filter((c) => c.kind === "judge");
    if (verdicts.length === 0) return null;
    return verdicts.reduce((sum, c) => sum + c.delta, 0);
  }

  /** The markers on each line. A probe (health checkpoint) is a disc in
   * its health colour — filled once the server verified it, hollow while
   * pending, dashed when the check failed — with a pill naming its grade and
   * score; a task's final tree gets a ring. A judge verdict is a diamond in
   * the participant's colour with the points it awarded. Any other scored
   * change is a small dot. */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  function drawPoints(u: any, members: ChartMember[]) {
    const ctx: CanvasRenderingContext2D = u.ctx;
    const dpr = window.devicePixelRatio || 1;
    const xs: number[] = u.data[0] ?? [];
    const { top } = u.bbox;
    ctx.save();
    ctx.font = `bold ${9 * dpr}px sans-serif`;
    for (let i = 0; i < members.length; i++) {
      const data: (number | null)[] = u.data[1 + i] ?? [];
      const color = seriesColor(i);
      // The right edge of the last pill on each row (above / below the
      // line), so pills of points seconds apart do not overprint.
      let aboveEnd = -Infinity;
      let belowEnd = -Infinity;
      const pill = (x: number, y: number, r: number, spec: Pill, preferAbove: boolean) => {
        // Above the marker unless that would leave the plot; a pill that
        // would overprint the previous one on its row is left to the tooltip.
        const half = pillWidth(ctx, spec, dpr) / 2;
        const roomAbove = y - r - 16 * dpr >= top;
        const fits = (row: boolean) => x - half > (row ? aboveEnd : belowEnd);
        // The preferred row, else the other one, else the tooltip has it.
        let above: boolean;
        if (preferAbove && roomAbove && fits(true)) above = true;
        else if (fits(false)) above = false;
        else if (roomAbove && fits(true)) above = true;
        else return;
        const [, right] = drawPill(ctx, x, above ? y - r - 3 * dpr : y + r + 3 * dpr, spec, dpr, above);
        if (above) aboveEnd = right + 3 * dpr;
        else belowEnd = right + 3 * dpr;
      };
      for (let idx = 0; idx < data.length; idx++) {
        const v = data[idx];
        if (v == null) continue;
        const meta = hookMeta.get(`${i}:${idx}`);
        if (!meta) continue;
        const x = u.valToPos(xs[idx], "x", true);
        const y = u.valToPos(v, "y", true);
        const cp = meta.checkpoint;
        const verdict = judgeDelta(meta);
        ctx.globalAlpha = 1;
        ctx.setLineDash([]);
        if (cp) {
          const level = LEVEL_COLORS[cp.level ?? "unknown"].fg;
          const r = 4 * dpr;
          ctx.beginPath();
          ctx.arc(x, y, r, 0, Math.PI * 2);
          ctx.lineWidth = 1.5 * dpr;
          if (cp.server_status === "ok") {
            ctx.fillStyle = level;
            ctx.fill();
            ctx.strokeStyle = color;
            ctx.stroke();
          } else if (cp.server_status === "pending") {
            ctx.fillStyle = "#ffffff";
            ctx.fill();
            ctx.strokeStyle = level;
            ctx.stroke();
          } else {
            ctx.setLineDash([2 * dpr, 2 * dpr]);
            ctx.fillStyle = "#ffffff";
            ctx.fill();
            ctx.strokeStyle = level;
            ctx.stroke();
            ctx.setLineDash([]);
          }
          if (cp.kind === "task_final") {
            // The task's final tree: a ring around the point.
            ctx.beginPath();
            ctx.arc(x, y, r + 2.5 * dpr, 0, Math.PI * 2);
            ctx.lineWidth = 1 * dpr;
            ctx.strokeStyle = color;
            ctx.stroke();
          }
          if (hookShowHealthLabels) {
            pill(
              x,
              y,
              r + (cp.kind === "task_final" ? 2.5 * dpr : 0),
              { kind: "health", grade: gradeOf(cp), score: cp.score ?? null, color: level },
              true,
            );
          }
          if (verdict !== null && hookShowPointLabels) {
            pill(x, y, r, { kind: "plain", text: `${verdict >= 0 ? "+" : "−"}${Math.abs(verdict)}`, color }, false);
          }
          continue;
        }
        if (verdict !== null) {
          // A judge verdict: a diamond in the participant's colour.
          const d = 5 * dpr;
          ctx.beginPath();
          ctx.moveTo(x, y - d);
          ctx.lineTo(x + d, y);
          ctx.lineTo(x, y + d);
          ctx.lineTo(x - d, y);
          ctx.closePath();
          ctx.fillStyle = color;
          ctx.fill();
          ctx.lineWidth = 1.5 * dpr;
          ctx.strokeStyle = "#ffffff";
          ctx.stroke();
          if (hookShowPointLabels) {
            pill(x, y, d, { kind: "plain", text: `${verdict >= 0 ? "+" : "−"}${Math.abs(verdict)}`, color }, true);
          }
          continue;
        }
        ctx.beginPath();
        ctx.arc(x, y, 2.5 * dpr, 0, Math.PI * 2);
        ctx.fillStyle = color;
        ctx.fill();
      }
    }
    ctx.restore();
  }

  function statusLabel(cp: HealthCheckpointView): string {
    switch (cp.server_status) {
      case "ok":
        return "verified";
      case "pending":
        return "pending verification";
      case "timeout":
        return "verification timed out";
      case "commit_missing":
        return "commit never arrived";
      case "unverified":
        return "unverified (history rewritten)";
      default:
        return "verification failed";
    }
  }

  function healthLines(cp: HealthCheckpointView): { label: string; value: string; tone?: string }[] {
    const lines: { label: string; value: string; tone?: string }[] = [];
    const m = cp.server?.metrics ?? cp.client?.metrics ?? null;
    const tone = LEVEL_COLORS[cp.level ?? "unknown"].fg;
    if (cp.score == null) {
      lines.push({
        label: "health",
        value: m && m.files === 0 ? "no code to score yet" : "not scored",
        tone,
      });
    } else {
      lines.push({ label: "health", value: `${healthLabel(cp)} · ${cp.level}`, tone });
    }
    if (m && m.files > 0) {
      lines.push({
        label: "duplication",
        value: `${m.duplication_pct == null ? "—" : `${m.duplication_pct.toFixed(1)}%`} · ${
          m.duplicated_lines ?? 0
        } of ${m.code_lines} lines · ${m.clones} clone${m.clones === 1 ? "" : "s"} · ${m.files} file${
          m.files === 1 ? "" : "s"
        }`,
      });
    }
    // Only what deserves attention: a check the server could not confirm,
    // a disagreement, an attempt to steer the scan.
    if (cp.server_status === "pending") {
      lines.push({ label: "check", value: "pending verification", tone: LEVEL_COLORS.amber.fg });
    } else if (cp.server_status !== "ok") {
      lines.push({ label: "check", value: statusLabel(cp), tone: LEVEL_COLORS.red.fg });
    }
    const flags = cp.flags ?? {};
    if (flags.score_mismatch && cp.client?.score != null && cp.server?.score != null) {
      lines.push({
        label: "mismatch",
        value: `client said ${formatScore(cp.client.score)}, server ${formatScore(cp.server.score)}`,
        tone: LEVEL_COLORS.red.fg,
      });
    }
    const badges = [
      flags.version_mismatch ? "jscpd version differs" : null,
      flags.task_mismatch ? "commit belongs to another task" : null,
      flags.late ? "reported after the task closed" : null,
      flags.history_rewritten ? "history rewritten" : null,
    ].filter((b): b is string => b !== null);
    if (badges.length > 0) {
      lines.push({ label: "flags", value: badges.join(", "), tone: LEVEL_COLORS.red.fg });
    }
    if (m && (m.ignore_markers > 0 || m.jscpd_config_present)) {
      lines.push({
        label: "note",
        value: [
          m.ignore_markers > 0 ? `${m.ignore_markers} file(s) with jscpd:ignore markers` : null,
          m.jscpd_config_present ? ".jscpd.json present (not applied)" : null,
        ]
          .filter(Boolean)
          .join("; "),
        tone: LEVEL_COLORS.amber.fg,
      });
    }
    const err = cp.server?.error ?? cp.client?.error;
    if (err) lines.push({ label: "error", value: err.slice(0, 120), tone: LEVEL_COLORS.red.fg });
    return lines;
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  function updateTooltip(u: any, members: ChartMember[]) {
    const idx: number | null | undefined = u.cursor?.idx;
    if (idx == null) {
      tip = null;
      return;
    }
    // The point(s) at this x, one per participant at most.
    const lines: { label: string; value: string; tone?: string }[] = [];
    for (let i = 0; i < members.length; i++) {
      const meta = hookMeta.get(`${i}:${idx}`);
      if (!meta) continue;
      const cp = meta.checkpoint;
      lines.push({ label: members[i].display_name, value: "", tone: seriesColor(i) });
      const range = taskAt(hookHealth?.players[members[i].player_id]?.task_ranges ?? [], meta.t);
      const task = cp?.task_title ?? range?.title ?? null;
      const what =
        cp?.kind === "task_final"
          ? "final tree"
          : cp?.kind === "probe"
            ? `check #${cp.probe_seq ?? "?"}`
            : judgeDelta(meta) !== null
              ? "judge verdict"
              : "points";
      lines.push({ label: fmtElapsed(meta.t), value: task ? `${what} · ${task}` : what });
      if (meta.delta !== null) {
        lines.push({
          label: "points",
          value: describePoints(meta),
          tone: meta.delta > 0 ? LEVEL_COLORS.green.fg : meta.delta < 0 ? LEVEL_COLORS.red.fg : undefined,
        });
      } else {
        lines.push({ label: "points", value: String(meta.total) });
      }
      if (cp) lines.push(...healthLines(cp));
    }
    if (lines.length === 0) {
      tip = null;
      return;
    }
    const x = u.cursor.left as number;
    const y = u.cursor.top as number;
    tip = { x, y, lines };
  }

  function membersOf(): ChartMember[] {
    return phase === "finished"
      ? reportMembers.map((m) => ({
          id: m.user_id,
          player_id: m.player_id ?? m.user_id,
          display_name: m.display_name,
        }))
      : leaderboard.map((e) => ({ id: e.player_id, player_id: e.player_id, display_name: e.display_name }));
  }

  // Build chart when chartEl is available and no instance exists yet.
  // uplotInstance is $state.raw so this effect re-runs when it is nulled out.
  $effect(() => {
    if (!browser || !chartEl || uplotInstance) return;
    const members = membersOf();
    if (members.length === 0) return;
    buildChart(members);
  });

  // Rebuild when the participant set changes while a chart already exists
  // (active phase).
  $effect(() => {
    if (!browser || !uplotInstance) return;
    const members = membersOf();
    const currentIds = members.map((m) => m.id).join(",");
    const chartIds = chartMembers.map((m) => m.id).join(",");
    if (phase === "active" && currentIds !== chartIds) {
      buildChart(members);
    }
  });

  $effect(() => {
    if (!browser || !uplotInstance) return;
    if (scoreHistory.length === 0 && !health) return;
    // Keep the full time axis so replay reveals the line against a fixed
    // frame: past the playhead, y is null (uPlot draws a gap) rather than
    // dropped, so the x scale stays [0, maxT] the whole way through.
    const cut = revealUntil;
    const { xs, series, meta } = buildChartData(chartMembers, scoreHistory, health, cut);
    hookHealth = health;
    hookMeta = meta;
    hookCut = cut;
    uplotInstance.setData([xs, ...series]);
  });

  onDestroy(() => {
    uplotInstance?.destroy();
  });

  const emptyMessage = $derived(
    phase === "finished" ? "No score history recorded." : "Waiting for first scores…",
  );
  const showEmpty = $derived(scoreHistory.length === 0 && !health);
</script>

<!-- min-height matches the uPlot canvas height so the card doesn't render
     collapsed and then jump ~240px once the chart mounts (or, in the empty
     state, never gain height at all — the absolute overlay contributed none).
     Audit UI-M4: blank/shifting areas on the session page. -->
{#if browser}
  <div class="relative min-h-[240px]">
    {#if health}
      <div class="absolute right-0 top-[-28px] z-10 flex items-center gap-[6px]">
        {#each [{ label: "Health labels", on: showHealthLabels, toggle: toggleHealthLabels, id: "health-labels-toggle" }, { label: "Points labels", on: showPointLabels, toggle: togglePointLabels, id: "point-labels-toggle" }] as t (t.id)}
          <button
            type="button"
            class="inline-flex items-center gap-[6px] rounded-full border border-brand-border bg-white px-[9px] py-[3px] text-[11px] font-medium transition-colors hover:bg-[#f4f8fe]"
            style="color: {t.on ? '#363636' : '#8fb4ec'};"
            aria-pressed={t.on}
            onclick={t.toggle}
            data-testid={t.id}
          >
            <span
              class="inline-block h-[8px] w-[8px] rounded-full border"
              style="background: {t.on ? '#3aa568' : 'transparent'}; border-color: {t.on ? '#3aa568' : '#8fb4ec'};"
              aria-hidden="true"
            ></span>
            {t.label}
          </button>
        {/each}
      </div>
    {/if}
    <div bind:this={chartEl} class="w-full"></div>
    {#if showEmpty}
      <div class="absolute inset-0 flex items-center justify-center">
        <p class="text-sm" style="color: #9aa6b2;">{emptyMessage}</p>
      </div>
    {/if}
    {#if tip}
      <div
        class="pointer-events-none absolute z-10 max-w-[280px] rounded-[8px] border border-brand-border bg-white p-2 text-[11px] leading-[1.35] shadow-lg"
        style="left: {Math.min(tip.x + 14, Math.max(0, (chartEl?.clientWidth ?? 600) - 290))}px; top: {Math.max(0, tip.y - 12)}px;"
        data-testid="health-tooltip"
      >
        {#each tip.lines as line, i (i)}
          {#if line.value === ""}
            <div class="font-semibold" style="color: {line.tone};">{line.label}</div>
          {:else}
            <div class="flex gap-2">
              <span class="shrink-0" style="color: #9ea7b6;">{line.label}</span>
              <span class="min-w-0 break-words" style={line.tone ? `color: ${line.tone};` : ""}>{line.value}</span>
            </div>
          {/if}
        {/each}
      </div>
    {/if}
  </div>
{:else}
  <div class="h-[240px] w-full rounded" style="background: #f4f8fe;"></div>
{/if}
