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
  import { LEVEL_COLORS, formatScore, seriesColor, shortSha } from "$lib/session-health";

  type Phase = "lobby" | "active" | "paused" | "finished";
  type Member = { id: string; player_id: string; display_name: string };

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
    /** Every participant's code-health history. `null` draws the score
     * chart exactly as before health existed: no right axis, no bands. */
    health?: SessionHealthPayload | null;
    /** Replay playhead (elapsed seconds): points past it are hidden while the
     * full time axis stays fixed, so the line draws in. `null` shows all. */
    revealUntil?: number | null;
  } = $props();

  let chartEl: HTMLDivElement | undefined = $state(undefined);
  // $state.raw: tracks the reference reactively but doesn't proxy the uPlot instance itself
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let uplotInstance = $state.raw<any>(null);
  let chartMembers: Member[] = [];
  let chartHasHealth = false;
  let chartBuilding = false; // reentry guard — prevents concurrent async buildChart calls

  // What the draw hooks read: the health payload behind the current data
  // and, per health series and data index, the checkpoint at that point.
  let hookHealth: SessionHealthPayload | null = null;
  let hookMeta: Map<string, HealthCheckpointView> = new Map();
  let hookCut: number | null = null;

  /** Tooltip for the health point under the cursor. */
  let tip = $state<{ x: number; y: number; lines: { label: string; value: string; tone?: string }[] } | null>(
    null,
  );

  const HEALTH_SCALE = "health";
  const SEPARATOR_LABEL_ROWS = 3;

  async function buildChart(members: Member[], withHealth: boolean) {
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

    // Format elapsed seconds as H:MM:SS (or M:SS) — values are seconds since
    // session start, NOT unix timestamps, so we disable uPlot time mode.
    const fmtElapsed = (v: number) => {
      const t = Math.max(0, Math.round(v));
      const h = Math.floor(t / 3600);
      const m = Math.floor((t % 3600) / 60);
      const s = t % 60;
      const mm = String(m).padStart(2, "0");
      const ss = String(s).padStart(2, "0");
      return h > 0 ? `${h}:${mm}:${ss}` : `${m}:${ss}`;
    };

    const series = [
      {
        label: "Time",
        // Disable time mode so uPlot does not format x as a unix timestamp
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        value: (_u: any, v: number) => fmtElapsed(v),
      },
      // Score lines: solid. The x axis is shared with the health points, so
      // a score series has gaps at health-only times — span them.
      ...members.map((p, i) => ({
        label: p.display_name,
        stroke: seriesColor(i),
        width: 2,
        spanGaps: true,
      })),
      // Health lines: dashed, same colour as the participant, on the right
      // scale. Points are drawn by the hook below (their shape says whether
      // the server has verified them), not by uPlot.
      ...(withHealth
        ? members.map((p, i) => ({
            label: `${p.display_name} · health`,
            scale: HEALTH_SCALE,
            stroke: seriesColor(i),
            width: 1.5,
            dash: [4, 3],
            spanGaps: true,
            points: { show: false },
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
            value: (_u: any, v: number | null) => (v == null ? "—" : v.toFixed(1)),
          }))
        : []),
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
    if (withHealth) {
      scales[HEALTH_SCALE] = { range: [0, 100] };
    }

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
      ...(withHealth
        ? [
            {
              scale: HEALTH_SCALE,
              side: 1,
              label: "Health",
              labelSize: 32,
              // eslint-disable-next-line @typescript-eslint/no-explicit-any
              values: (_u: any, vals: number[]) => vals.map((v) => String(Math.round(v))),
              stroke: "#8fb4ec",
              font: "11px sans-serif",
              labelFont: "bold 11px sans-serif",
              grid: { show: false },
              ticks: { stroke: "#e0e8f0", width: 1 },
              space: 30,
            },
          ]
        : []),
    ];

    try {
      uplotInstance = new uPlot(
        {
          width,
          height: 240,
          series,
          scales,
          hooks: {
            // Level bands behind everything, on the health scale.
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
            drawClear: [(u: any) => drawBands(u)],
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
            draw: [(u: any) => {
              drawZeroLine(u);
              drawTaskSeparators(u, members);
              drawHealthPoints(u, members);
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
      chartHasHealth = withHealth;
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

  /** Faint green / amber / red bands on the health scale. */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  function drawBands(u: any) {
    if (!hookHealth || !u.scales[HEALTH_SCALE]) return;
    const { green_min, amber_min } = hookHealth.thresholds;
    const { left, width: w, top, height: h } = u.bbox;
    const ctx: CanvasRenderingContext2D = u.ctx;
    const yOf = (v: number) => Math.min(top + h, Math.max(top, u.valToPos(v, HEALTH_SCALE, true)));
    const bands: [number, number, string][] = [
      [green_min, 100, LEVEL_COLORS.green.bg],
      [amber_min, green_min, LEVEL_COLORS.amber.bg],
      [0, amber_min, LEVEL_COLORS.red.bg],
    ];
    ctx.save();
    for (const [lo, hi, fill] of bands) {
      const yTop = yOf(hi);
      const yBot = yOf(lo);
      ctx.fillStyle = fill;
      ctx.globalAlpha = 0.5;
      ctx.fillRect(left, yTop, w, Math.max(0, yBot - yTop));
    }
    ctx.restore();
  }

  /** Vertical separators where each participant's tasks start, tinted
   * with the participant's colour and labelled with the task title. */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  function drawTaskSeparators(u: any, members: Member[]) {
    if (!hookHealth) return;
    const { left, width: w, top, height: h } = u.bbox;
    const ctx: CanvasRenderingContext2D = u.ctx;
    const dpr = window.devicePixelRatio || 1;
    ctx.save();
    ctx.font = `${10 * dpr}px sans-serif`;
    ctx.textBaseline = "top";
    members.forEach((m, i) => {
      const ranges = hookHealth?.players[m.player_id]?.task_ranges ?? [];
      const color = seriesColor(i);
      // The right edge of the last label drawn on this participant's row:
      // a title that would overprint it is left to the tooltip.
      let labelEnd = -Infinity;
      for (const r of ranges) {
        const t = r.start_t;
        if (t == null || (hookCut != null && t > hookCut)) continue;
        // The first task starts at t = 0, on the axis itself: keep a marker
        // that lands a sub-pixel outside the plot instead of dropping it.
        const x = Math.min(Math.max(u.valToPos(t, "x", true), left), left + w);
        if (x < left - 1 || x > left + w + 1) continue;
        ctx.strokeStyle = color;
        ctx.globalAlpha = 0.55;
        ctx.lineWidth = 1;
        ctx.setLineDash([2, 3]);
        ctx.beginPath();
        ctx.moveTo(x, top);
        ctx.lineTo(x, top + h);
        ctx.stroke();
        const title = (r.title ?? "task").slice(0, 28);
        // Stack labels per participant so two players' markers do not
        // overprint; past a few participants, the tooltip carries the title.
        // Along the row, a label is skipped when the previous one is still
        // under it (tasks closed seconds apart).
        const labelX = x + 3 * dpr;
        if (i < SEPARATOR_LABEL_ROWS && labelX >= labelEnd) {
          ctx.setLineDash([]);
          ctx.globalAlpha = 0.9;
          ctx.fillStyle = color;
          ctx.fillText(title, labelX, top + 2 * dpr + i * 12 * dpr);
          labelEnd = labelX + ctx.measureText(title).width + 8 * dpr;
        }
      }
    });
    ctx.restore();
  }

  /** The health points: filled when the server verified them, hollow while
   * pending, a dashed ring when the check failed or was never possible. */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  function drawHealthPoints(u: any, members: Member[]) {
    if (!hookHealth || !u.scales[HEALTH_SCALE]) return;
    const ctx: CanvasRenderingContext2D = u.ctx;
    const dpr = window.devicePixelRatio || 1;
    const r = 3.5 * dpr;
    ctx.save();
    for (let i = 0; i < members.length; i++) {
      const sIdx = 1 + members.length + i;
      const data: (number | null)[] = u.data[sIdx] ?? [];
      const xs: number[] = u.data[0] ?? [];
      const color = seriesColor(i);
      for (let idx = 0; idx < data.length; idx++) {
        const v = data[idx];
        if (v == null) continue;
        const cp = hookMeta.get(`${sIdx}:${idx}`);
        const x = u.valToPos(xs[idx], "x", true);
        const y = u.valToPos(v, HEALTH_SCALE, true);
        ctx.beginPath();
        ctx.arc(x, y, r, 0, Math.PI * 2);
        ctx.lineWidth = 1.5 * dpr;
        ctx.strokeStyle = color;
        const status = cp?.server_status ?? "pending";
        if (status === "ok") {
          ctx.setLineDash([]);
          ctx.fillStyle = color;
          ctx.fill();
        } else if (status === "pending") {
          ctx.setLineDash([]);
          ctx.fillStyle = "#ffffff";
          ctx.fill();
          ctx.stroke();
        } else {
          ctx.setLineDash([2 * dpr, 2 * dpr]);
          ctx.fillStyle = "#ffffff";
          ctx.fill();
          ctx.stroke();
        }
        if (cp?.kind === "task_final") {
          // The task's final tree: a ring around the point.
          ctx.setLineDash([]);
          ctx.beginPath();
          ctx.arc(x, y, r + 2.5 * dpr, 0, Math.PI * 2);
          ctx.lineWidth = 1 * dpr;
          ctx.stroke();
        }
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

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  function updateTooltip(u: any, members: Member[]) {
    const idx: number | null | undefined = u.cursor?.idx;
    if (idx == null || !hookHealth) {
      tip = null;
      return;
    }
    // The health point(s) at this x, one per participant at most.
    const lines: { label: string; value: string; tone?: string }[] = [];
    for (let i = 0; i < members.length; i++) {
      const sIdx = 1 + members.length + i;
      const cp = hookMeta.get(`${sIdx}:${idx}`);
      if (!cp) continue;
      const flags = cp.flags ?? {};
      const badges = [
        flags.score_mismatch ? "score mismatch" : null,
        flags.version_mismatch ? "version mismatch" : null,
        flags.task_mismatch ? "task mismatch" : null,
        flags.late ? "late" : null,
        flags.history_rewritten ? "history rewritten" : null,
      ].filter((b): b is string => b !== null);
      const m = cp.server?.metrics ?? cp.client?.metrics ?? null;
      lines.push({ label: members[i].display_name, value: "", tone: seriesColor(i) });
      lines.push({
        label: "task",
        value: `${cp.task_title ?? "—"}${cp.kind === "task_final" ? " (final tree)" : ""}`,
      });
      if (cp.kind === "probe") lines.push({ label: "probe", value: `#${cp.probe_seq ?? "?"}` });
      lines.push({ label: "time", value: new Date(cp.created_at).toLocaleTimeString() });
      lines.push({
        label: "server score",
        value: cp.server ? `${formatScore(cp.server.score)} ${cp.server.grade ?? ""}`.trim() : "—",
        tone: LEVEL_COLORS[cp.server?.level ?? "unknown"].fg,
      });
      lines.push({
        label: "client score",
        value: cp.client ? `${formatScore(cp.client.score)} ${cp.client.grade ?? ""}`.trim() : "—",
        tone: LEVEL_COLORS[cp.client?.level ?? "unknown"].fg,
      });
      if (m) {
        lines.push({
          label: "duplication",
          value: `${m.duplication_pct == null ? "—" : `${m.duplication_pct.toFixed(1)}%`} · ${
            m.duplicated_lines ?? 0
          } lines · ${m.clones} clones`,
        });
        lines.push({ label: "files", value: `${m.files} · ${m.code_lines} lines of code` });
        if (m.ignore_markers > 0 || m.jscpd_config_present) {
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
      }
      lines.push({ label: "commit", value: shortSha(cp.commit) });
      lines.push({
        label: "check",
        value: statusLabel(cp),
        tone: cp.server_status === "ok" ? LEVEL_COLORS.green.fg : LEVEL_COLORS.amber.fg,
      });
      if (badges.length > 0) {
        lines.push({ label: "flags", value: badges.join(", "), tone: LEVEL_COLORS.red.fg });
      }
      const err = cp.server?.error ?? cp.client?.error;
      if (err) lines.push({ label: "error", value: err.slice(0, 120) });
    }
    if (lines.length === 0) {
      tip = null;
      return;
    }
    const x = u.cursor.left as number;
    const y = u.cursor.top as number;
    tip = { x, y, lines };
  }

  function membersOf(): Member[] {
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
    buildChart(members, health != null);
  });

  // Rebuild when the participant set changes while a chart already exists
  // (active phase), or when health data first appears (the series set
  // grows by one line per participant).
  $effect(() => {
    if (!browser || !uplotInstance) return;
    const members = membersOf();
    const currentIds = members.map((m) => m.id).join(",");
    const chartIds = chartMembers.map((m) => m.id).join(",");
    const wantHealth = health != null;
    if ((phase === "active" && currentIds !== chartIds) || wantHealth !== chartHasHealth) {
      buildChart(members, wantHealth);
    }
  });

  $effect(() => {
    if (!browser || !uplotInstance) return;
    if (scoreHistory.length === 0 && !health) return;
    // Keep the full time axis so replay reveals the line against a fixed
    // frame: past the playhead, y is null (uPlot draws a gap) rather than
    // dropped, so the x scale stays [0, maxT] the whole way through.
    const cut = revealUntil;
    const members = chartMembers;
    const nMembers = members.length;

    // One x axis for scores and health: the union of their times, sorted.
    const times = new Set<number>();
    for (const pt of scoreHistory) times.add(pt.t);
    const healthAt = new Map<string, HealthCheckpointView[]>(); // `${member}:${t}` → checkpoints
    if (health && chartHasHealth) {
      members.forEach((m, i) => {
        const cps = health.players[m.player_id]?.checkpoints ?? [];
        for (const cp of cps) {
          if (cp.t == null || cp.score == null) continue;
          times.add(cp.t);
          const key = `${i}:${cp.t}`;
          const list = healthAt.get(key) ?? [];
          list.push(cp);
          healthAt.set(key, list);
        }
      });
    }
    const xs = [...times].sort((a, b) => a - b);
    const scoreAt = new Map<number, Record<string, number>>();
    for (const pt of scoreHistory) scoreAt.set(pt.t, pt.scores);

    const scoreSeries: (number | null)[][] = members.map((m) =>
      xs.map((t) => {
        if (cut != null && t > cut) return null;
        const scores = scoreAt.get(t);
        return scores ? (scores[m.id] ?? 0) : null;
      }),
    );
    const meta = new Map<string, HealthCheckpointView>();
    const healthSeries: (number | null)[][] = chartHasHealth
      ? members.map((_m, i) =>
          xs.map((t, idx) => {
            if (cut != null && t > cut) return null;
            const cps = healthAt.get(`${i}:${t}`);
            if (!cps || cps.length === 0) return null;
            // Several checkpoints at one instant: the last one wins.
            const cp = cps[cps.length - 1];
            meta.set(`${1 + nMembers + i}:${idx}`, cp);
            return cp.score ?? null;
          }),
        )
      : [];
    hookHealth = chartHasHealth ? health : null;
    hookMeta = meta;
    hookCut = cut;
    uplotInstance.setData([xs, ...scoreSeries, ...healthSeries]);
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
