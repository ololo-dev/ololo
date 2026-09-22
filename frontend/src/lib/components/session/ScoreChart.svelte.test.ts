import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/svelte";
import ScoreChart from "./ScoreChart.svelte";
import type { LeaderboardEntry, MemberInfo, ScoreHistoryPoint } from "$lib/types/arena";

const ALICE_MEMBER: MemberInfo = {
  user_id: "p1",
  display_name: "Alice",
  joined_at: "",
  avatar_url: null,
  fingerprint: null,
  username: null,
  agent_display_name: null,
};

const ALICE_ENTRY: LeaderboardEntry = {
  player_id: "p1",
  display_name: "Alice",
  agent_display_name: null,
  total_points: 5,
  tests_passed: 1,
  total_wall_ms: 0,
};

describe("ScoreChart", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  // Regression: buildChart defers its width read to requestAnimationFrame.
  // A component unmounted while that frame was pending (a test tearing the
  // session page down, a fast navigation) left `chartEl` null, and the
  // callback threw "Cannot read properties of null (reading 'clientWidth')"
  // as an uncaught exception outside any await — vitest reported it as an
  // unhandled error and the whole frontend CI job went red.
  it("survives unmounting before the layout frame fires", async () => {
    const frames: FrameRequestCallback[] = [];
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((cb) => {
      frames.push(cb);
      return frames.length;
    });

    const { unmount } = render(ScoreChart, {
      phase: "active",
      leaderboard: [ALICE_ENTRY],
      reportMembers: [],
      scoreHistory: [{ t: 0, scores: { p1: 5 } }],
    });

    // buildChart awaits the uPlot import before it schedules the frame.
    await vi.waitFor(() => expect(frames.length).toBeGreaterThan(0));

    unmount();
    expect(() => frames.forEach((cb) => cb(performance.now()))).not.toThrow();
  });

  it("finished phase with populated scoreHistory shows no empty message", () => {
    const scoreHistory: ScoreHistoryPoint[] = [
      { t: 0, scores: { p1: 0 } },
      { t: 10, scores: { p1: 5 } },
    ];

    render(ScoreChart, {
      phase: "finished",
      leaderboard: [],
      reportMembers: [ALICE_MEMBER],
      scoreHistory,
    });

    expect(screen.queryByText("No score history recorded.")).toBeNull();
  });

  it("active phase with populated scoreHistory shows no empty message", () => {
    const scoreHistory: ScoreHistoryPoint[] = [{ t: 0, scores: { p1: 5 } }];

    render(ScoreChart, {
      phase: "active",
      leaderboard: [ALICE_ENTRY],
      reportMembers: [],
      scoreHistory,
    });

    expect(screen.queryByText("Waiting for first scores…")).toBeNull();
  });

  it('finished phase with empty scoreHistory shows "No score history recorded."', () => {
    render(ScoreChart, {
      phase: "finished",
      leaderboard: [],
      reportMembers: [ALICE_MEMBER],
      scoreHistory: [],
    });

    expect(screen.queryByText("No score history recorded.")).not.toBeNull();
  });

  it('active phase with empty scoreHistory shows "Waiting for first scores…"', () => {
    render(ScoreChart, {
      phase: "active",
      leaderboard: [ALICE_ENTRY],
      reportMembers: [],
      scoreHistory: [],
    });

    expect(screen.queryByText("Waiting for first scores…")).not.toBeNull();
  });

  it("does not read sessionReport.timeline for the chart", () => {
    // scoreHistory is empty — chart must show the empty message regardless of
    // sessionReport.timeline contents. Guards against re-introducing the
    // REST timeline fold that previously fed the chart from sessionReport.
    // sessionReport is no longer a ScoreChart prop; the timeline stays REST-only.
    render(ScoreChart, {
      phase: "finished",
      leaderboard: [],
      reportMembers: [],
      scoreHistory: [],
    });

    expect(screen.queryByText("No score history recorded.")).not.toBeNull();
  });
});

// ── Code health on the same chart ──────────────────────────────────────────

import type { HealthCheckpointView, SessionHealthPayload } from "$lib/types/arena";

function checkpoint(
  over: Partial<HealthCheckpointView> & { id: string; t: number },
): HealthCheckpointView {
  return {
    kind: "probe",
    probe_id: over.id,
    probe_seq: 1,
    task_id: "task-1",
    task_title: "Widget",
    commit: "abcdef1234567890",
    created_at: "2026-09-22T10:00:00Z",
    client: null,
    server: null,
    server_status: "ok",
    flags: {},
    score: 80,
    level: "green",
    ...over,
  };
}

const HEALTH: SessionHealthPayload = {
  thresholds: { green_min: 70, amber_min: 55 },
  players: {
    p1: {
      checkpoints: [
        checkpoint({ id: "c1", t: 2, score: 80, server_status: "ok" }),
        checkpoint({ id: "c2", t: 6, score: 62, level: "amber", server_status: "pending" }),
        checkpoint({ id: "c3", t: 9, score: 40, level: "red", server_status: "failed" }),
      ],
      task_ranges: [
        { task_id: "task-1", title: "Widget", ordinal: 0, start_commit: "a", start_t: 1 },
      ],
    },
  },
};

/** A 2D context that records every call, so the draw hooks can be checked
 *  without a real canvas (jsdom has none). */
function recordingContext() {
  const calls: { name: string; args: unknown[] }[] = [];
  const ctx = new Proxy(
    {},
    {
      get: (_t, key: string) => {
        if (key === "measureText") return () => ({ width: 0 });
        if (key === "canvas") return { width: 600, height: 240 };
        if (key === "getImageData") return () => ({ data: new Uint8ClampedArray(4) });
        return (...args: unknown[]) => {
          calls.push({ name: key, args });
          return undefined;
        };
      },
      set: () => true,
    },
  );
  return { ctx: ctx as unknown as CanvasRenderingContext2D, calls };
}

describe("ScoreChart with health", () => {
  // jsdom has no Path2D; uPlot builds every series path with it once a real
  // (stubbed) 2D context lets the draw run at all.
  beforeEach(() => {
    vi.stubGlobal(
      "Path2D",
      class {
        // Every path method is a no-op.
        constructor() {
          return new Proxy(this, { get: () => () => undefined });
        }
      },
    );
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("adds a dashed health series per participant, the bands, the separators and the points", async () => {
    const { ctx, calls } = recordingContext();
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(() => ctx);

    const { container } = render(ScoreChart, {
      phase: "active",
      leaderboard: [ALICE_ENTRY],
      reportMembers: [],
      scoreHistory: [
        { t: 0, scores: { p1: 0 } },
        { t: 10, scores: { p1: 5 } },
      ],
      health: HEALTH,
    });

    // One legend entry per series: Time, Alice, Alice · health.
    await vi.waitFor(() => {
      const labels = [...container.querySelectorAll(".u-legend .u-series")].map(
        (el) => el.textContent ?? "",
      );
      expect(labels.some((l) => l.includes("Alice · health"))).toBe(true);
    });
    await vi.waitFor(() => {
      // Three level bands behind the plot.
      expect(calls.filter((c) => c.name === "fillRect").length).toBeGreaterThanOrEqual(3);
      // Three health points drawn as circles: filled (ok), hollow (pending), dashed (failed).
      expect(calls.filter((c) => c.name === "arc").length).toBeGreaterThanOrEqual(3);
      // The failed point's dashed ring.
      expect(
        calls.some(
          (c) =>
            c.name === "setLineDash" &&
            Array.isArray(c.args[0]) &&
            (c.args[0] as number[]).length === 2 &&
            (c.args[0] as number[])[0] === (c.args[0] as number[])[1] &&
            (c.args[0] as number[])[0] > 0 &&
            (c.args[0] as number[])[0] !== 4,
        ),
      ).toBe(true);
      // The task separator label.
      expect(calls.some((c) => c.name === "fillText" && c.args[0] === "Widget")).toBe(true);
    });
    expect(screen.queryByText("Waiting for first scores…")).toBeNull();
  });

  it("without health data the chart draws exactly as before: no health series, no bands", async () => {
    const { ctx, calls } = recordingContext();
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(() => ctx);
    const { container } = render(ScoreChart, {
      phase: "active",
      leaderboard: [ALICE_ENTRY],
      reportMembers: [],
      scoreHistory: [
        { t: 0, scores: { p1: 5 } },
        { t: 5, scores: { p1: 7 } },
      ],
    });
    await vi.waitFor(() => {
      const labels = [...container.querySelectorAll(".u-legend .u-series")].map(
        (el) => el.textContent ?? "",
      );
      expect(labels.some((l) => l.includes("Alice"))).toBe(true);
    });
    await vi.waitFor(() => expect(calls.some((c) => c.name === "stroke")).toBe(true));
    expect(
      [...container.querySelectorAll(".u-legend .u-series")].some((el) =>
        (el.textContent ?? "").includes("health"),
      ),
    ).toBe(false);
    expect(calls.filter((c) => c.name === "fillRect").length).toBe(0);
  });

  it("health points alone (no scores yet) still draw the chart", async () => {
    const { ctx } = recordingContext();
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(() => ctx);
    render(ScoreChart, {
      phase: "active",
      leaderboard: [ALICE_ENTRY],
      reportMembers: [],
      scoreHistory: [],
      health: HEALTH,
    });
    expect(screen.queryByText("Waiting for first scores…")).toBeNull();
  });
});
