import { describe, expect, it } from "vitest";
import { indicatorFor, levelOf, upsertCheckpoint } from "./session-health";
import type { HealthCheckpointView, SessionHealthPayload } from "$lib/types/arena";

function cp(over: Partial<HealthCheckpointView> & { id: string }): HealthCheckpointView {
  return {
    kind: "probe",
    probe_seq: 1,
    task_id: "t",
    commit: "abc",
    created_at: "2026-09-22T10:00:00Z",
    t: 1,
    server_status: "ok",
    flags: {},
    score: 70,
    level: "green",
    ...over,
  };
}

const T = { green_min: 70, amber_min: 55 };

describe("session-health", () => {
  it("levels follow the thresholds, inclusive at the bottom of each band", () => {
    expect(levelOf(70, T)).toBe("green");
    expect(levelOf(69.9, T)).toBe("amber");
    expect(levelOf(55, T)).toBe("amber");
    expect(levelOf(54.9, T)).toBe("red");
    expect(levelOf(null, T)).toBe("unknown");
  });

  it("upsert replaces by id, keeps time order and never mutates the input", () => {
    const base: SessionHealthPayload = {
      thresholds: T,
      players: {
        p1: { checkpoints: [cp({ id: "a", t: 1 }), cp({ id: "b", t: 5 })], task_ranges: [] },
      },
    };
    const next = upsertCheckpoint(base, "p1", cp({ id: "c", t: 3 }));
    expect(next.players.p1.checkpoints.map((c) => c.id)).toEqual(["a", "c", "b"]);
    expect(base.players.p1.checkpoints.map((c) => c.id)).toEqual(["a", "b"]);
    const replaced = upsertCheckpoint(next, "p1", cp({ id: "b", t: 5, score: 10 }));
    expect(replaced.players.p1.checkpoints.find((c) => c.id === "b")?.score).toBe(10);
    expect(replaced.players.p1.checkpoints.length).toBe(3);
    const fresh = upsertCheckpoint(null, "p9", cp({ id: "z" }));
    expect(fresh.players.p9.checkpoints.length).toBe(1);
    expect(fresh.thresholds).toEqual(T);
  });

  it("the indicator prefers verified points and reports the trend", () => {
    const payload: SessionHealthPayload = {
      thresholds: T,
      players: {
        p1: {
          checkpoints: [
            cp({ id: "a", t: 1, score: 60 }),
            cp({ id: "b", t: 2, score: 80 }),
            cp({ id: "c", t: 3, score: 20, server_status: "pending" }),
          ],
          task_ranges: [],
        },
      },
    };
    const ind = indicatorFor(payload, "p1")!;
    expect(ind.score).toBe(80);
    expect(ind.level).toBe("green");
    expect(ind.trend).toBe("up");
    expect(ind.verified).toBe(true);
    expect(indicatorFor(payload, "nobody")).toBeNull();
    // Only client numbers so far: shown, but marked unverified.
    const pending: SessionHealthPayload = {
      thresholds: T,
      players: {
        p1: {
          checkpoints: [cp({ id: "c", score: 50, server_status: "pending" })],
          task_ranges: [],
        },
      },
    };
    const p = indicatorFor(pending, "p1")!;
    expect(p.verified).toBe(false);
    expect(p.level).toBe("red");
    expect(p.trend).toBeNull();
  });
});
