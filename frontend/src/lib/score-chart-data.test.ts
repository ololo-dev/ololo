import { describe, expect, it } from "vitest";
import { buildChartData, describeChange, describePoints, taskAt } from "./score-chart-data";
import type { HealthCheckpointView, SessionHealthPayload } from "$lib/types/arena";

const ALICE = { id: "u1", player_id: "p1", display_name: "Alice" };
const BOB = { id: "u2", player_id: "p2", display_name: "Bob" };

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
        checkpoint({ id: "c1", t: 4 }),
        checkpoint({ id: "c2", t: 10, score: 62, level: "amber" }),
        checkpoint({ id: "c3", t: 25, kind: "task_final" }),
      ],
      task_ranges: [
        {
          task_id: "task-1",
          title: "Widget",
          ordinal: 0,
          start_commit: "a",
          start_t: 0,
          end_t: 25,
        },
        { task_id: "task-2", title: "Gadget", ordinal: 1, start_commit: "b", start_t: 25 },
      ],
    },
  },
};

describe("buildChartData", () => {
  it("carries the last total between score events so every probe sits on the line", () => {
    const { xs, series, meta } = buildChartData(
      [ALICE],
      [
        { t: 0, scores: { p1: 0 } },
        { t: 10, scores: { p1: 5 }, changes: [{ player_id: "p1", delta: 5, kind: "probe" }] },
        { t: 20, scores: { p1: 3 } },
      ],
      HEALTH,
    );
    expect(xs).toEqual([0, 4, 10, 20, 25]);
    expect(series[0]).toEqual([0, 0, 5, 3, 3]);
    // The probe at t = 4 changed nothing: on the line, no delta.
    expect(meta.get("0:1")).toMatchObject({
      t: 4,
      total: 0,
      delta: null,
      checkpoint: { id: "c1" },
    });
    // The probe at t = 10 scored: the server said why.
    expect(meta.get("0:2")).toMatchObject({ total: 5, delta: 5, checkpoint: { id: "c2" } });
    expect(meta.get("0:2")?.changes).toHaveLength(1);
    // A live point without changes: the delta is the totals' difference.
    expect(meta.get("0:3")).toMatchObject({ total: 3, delta: -2, changes: [], checkpoint: null });
    // The task's final tree, no points moved.
    expect(meta.get("0:4")).toMatchObject({
      total: 3,
      delta: null,
      checkpoint: { kind: "task_final" },
    });
    // t = 0 scored nothing and has no checkpoint: no meta, no marker.
    expect(meta.has("0:0")).toBe(false);
  });

  it("keys scores by user id or player id and changes by either too", () => {
    const { series, meta } = buildChartData(
      [ALICE, BOB],
      [
        {
          t: 5,
          scores: { u1: 7, p2: 2 },
          changes: [{ player_id: "u1", delta: 7, kind: "judge", label: "Tests" }],
        },
      ],
      null,
    );
    expect(series).toEqual([[7], [2]]);
    expect(meta.get("0:0")?.changes[0]?.label).toBe("Tests");
    expect(meta.get("1:0")).toMatchObject({ delta: 2, changes: [] });
  });

  it("hides points past the reveal cut but keeps the axis", () => {
    const { xs, series } = buildChartData(
      [ALICE],
      [
        { t: 0, scores: { p1: 1 } },
        { t: 30, scores: { p1: 9 } },
      ],
      null,
      10,
    );
    expect(xs).toEqual([0, 30]);
    expect(series[0]).toEqual([1, null]);
  });

  it("draws the line from zero when only checkpoints exist", () => {
    const { xs, series, meta } = buildChartData([ALICE], [], HEALTH);
    expect(xs).toEqual([4, 10, 25]);
    expect(series[0]).toEqual([0, 0, 0]);
    expect(meta.size).toBe(3);
  });
});

describe("taskAt", () => {
  it("finds the range covering an instant, the open one at the end", () => {
    const ranges = HEALTH.players.p1.task_ranges;
    expect(taskAt(ranges, 3)?.title).toBe("Widget");
    expect(taskAt(ranges, 40)?.title).toBe("Gadget");
    expect(taskAt([], 3)).toBeNull();
  });
});

describe("describeChange / describePoints", () => {
  it("names each kind of change with its sign", () => {
    expect(describeChange({ player_id: "p1", delta: 10, kind: "probe" })).toBe("+10 check passed");
    expect(describeChange({ player_id: "p1", delta: -5, kind: "probe" })).toBe("−5 check failed");
    expect(describeChange({ player_id: "p1", delta: 20, kind: "completion_bonus" })).toBe(
      "+20 task bonus",
    );
    expect(describeChange({ player_id: "p1", delta: 7, kind: "health_bonus" })).toBe(
      "+7 health bonus",
    );
    expect(describeChange({ player_id: "p1", delta: -12, kind: "similarity_penalty" })).toBe(
      "−12 similarity penalty",
    );
    expect(
      describeChange({ player_id: "p1", delta: 14, kind: "judge", label: "Performance" }),
    ).toBe("+14 judge · Performance");
  });

  it("describes the points line of the tooltip", () => {
    expect(describePoints({ t: 1, total: 39, delta: null, changes: [], checkpoint: null })).toBe(
      "39",
    );
    expect(describePoints({ t: 1, total: 39, delta: -2, changes: [], checkpoint: null })).toBe(
      "39 (−2)",
    );
    expect(
      describePoints({
        t: 1,
        total: 39,
        delta: 24,
        changes: [
          { player_id: "p1", delta: 10, kind: "probe" },
          { player_id: "p1", delta: 14, kind: "judge", label: "Performance" },
        ],
        checkpoint: null,
      }),
    ).toBe("39 (+10 check passed, +14 judge · Performance)");
  });
});
