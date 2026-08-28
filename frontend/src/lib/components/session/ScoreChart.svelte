<script lang="ts">
  import { browser } from "$app/environment";
  import { onDestroy } from "svelte";
  import type { LeaderboardEntry, MemberInfo, ScoreHistoryPoint } from "$lib/types/arena";
  import type { SessionReportResponse } from "$lib/api";

  type Phase = "lobby" | "active" | "paused" | "finished";

  let {
    phase,
    leaderboard,
    reportMembers,
    sessionReport,
    scoreHistory,
    revealUntil = null,
  }: {
    phase: Phase;
    leaderboard: LeaderboardEntry[];
    reportMembers: MemberInfo[];
    sessionReport: SessionReportResponse | null;
    scoreHistory: ScoreHistoryPoint[];
    /** Replay playhead (elapsed seconds): points past it are hidden while the
     * full time axis stays fixed, so the line draws in. `null` shows all. */
    revealUntil?: number | null;
  } = $props();

  let chartEl: HTMLDivElement | undefined = $state(undefined);
  // $state.raw: tracks the reference reactively but doesn't proxy the uPlot instance itself
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let uplotInstance = $state.raw<any>(null);
  let chartParticipantIds: string[] = [];
  let chartBuilding = false; // reentry guard — prevents concurrent async buildChart calls

  const SERIES_COLORS = ["#6be597", "#fb341c", "#8fb4ec", "#f5a623", "#bd10e0", "#4a90e2", "#7ed321"];
  function seriesColor(idx: number): string {
    return SERIES_COLORS[idx % SERIES_COLORS.length];
  }

  async function buildChart(members: { id: string; display_name: string }[]) {
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
    // clientWidth reflects the actual rendered width (not 0).
    const width = await new Promise<number>((resolve) => {
      requestAnimationFrame(() => resolve(chartEl!.clientWidth || 600));
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
      ...members.map((p, i) => ({ label: p.display_name, stroke: seriesColor(i), width: 2 })),
    ];

    const chartData: number[][] = [[], ...members.map(() => [])];

    try {
      uplotInstance = new uPlot(
        {
          width,
          height: 240,
          series,
          scales: {
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
          },
          hooks: {
            // Draw a subtle dashed zero reference line
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
            draw: [(u: any) => {
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
            }],
          },
          axes: [
            {
              label: "Time (s)",
              labelSize: 18,
              // eslint-disable-next-line @typescript-eslint/no-explicit-any
              values: (_u: any, vals: number[]) => vals.map((v: number) => {
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
          ],
        },
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        chartData as any,
        chartEl,
      );
      chartParticipantIds = members.map((m) => m.id);
    } catch (err) {
      console.error("[chart] uPlot init failed:", err);
    } finally {
      chartBuilding = false;
    }
  }

  // Build chart when chartEl is available and no instance exists yet.
  // uplotInstance is $state.raw so this effect re-runs when it is nulled out.
  $effect(() => {
    if (!browser || !chartEl || uplotInstance) return;
    const members = phase === "finished"
      ? reportMembers.map((m) => ({ id: m.user_id, display_name: m.display_name }))
      : leaderboard.map((e) => ({ id: e.player_id, display_name: e.display_name }));
    if (members.length === 0) return;
    buildChart(members);
  });

  // Rebuild when the leaderboard player set changes while a chart already exists.
  // Only relevant for the active phase; finished sessions source series from reportMembers.
  $effect(() => {
    if (!browser || phase !== "active" || !uplotInstance) return;
    const currentIds = leaderboard.map((e) => e.player_id).join(",");
    const chartIds = chartParticipantIds.join(",");
    if (currentIds !== chartIds) {
      buildChart(leaderboard.map((e) => ({ id: e.player_id, display_name: e.display_name })));
    }
  });

  $effect(() => {
    if (!browser || !uplotInstance || scoreHistory.length === 0) return;
    // Keep the full time axis so replay reveals the line against a fixed
    // frame: past the playhead, y is null (uPlot draws a gap) rather than
    // dropped, so the x scale stays [0, maxT] the whole way through.
    const cut = revealUntil;
    const times: number[] = scoreHistory.map((pt) => pt.t);
    const seriesData: (number | null)[][] = chartParticipantIds.map((id) =>
      scoreHistory.map((pt) => (cut != null && pt.t > cut ? null : (pt.scores[id] ?? 0))),
    );
    uplotInstance.setData([times, ...seriesData]);
  });

  onDestroy(() => {
    uplotInstance?.destroy();
  });

  const emptyMessage = $derived(
    phase === "finished" ? "No score history recorded." : "Waiting for first scores…",
  );
  const showEmpty = $derived(scoreHistory.length === 0);
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
  </div>
{:else}
  <div class="h-[240px] w-full rounded" style="background: #f4f8fe;"></div>
{/if}
