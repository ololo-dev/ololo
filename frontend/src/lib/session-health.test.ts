import { describe, expect, it } from "vitest";
import {
  gradeOf,
  indicatorFor,
  levelOf,
  testsLines,
  testsVerdict,
  upsertCheckpoint,
} from "./session-health";
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
    expect(ind.grade).toBe("B");
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
    expect(p.grade).toBe("D");
    expect(p.level).toBe("red");
    expect(p.trend).toBeNull();
  });
});

describe("the project's tests in a checkpoint", () => {
  it("the grade is the composed score's own", () => {
    const composed = cp({
      id: "a",
      score: 68.4,
      grade: "C",
      server: {
        status: "ok",
        score: 80,
        grade: "B",
        level: "green",
        jscpd_version: "0.1.17",
        duration_ms: 1,
      },
    });
    expect(gradeOf(composed)).toBe("C");
    expect(gradeOf({ ...composed, grade: null })).toBe("B");
  });

  it("says what the run counted, what it scored and where its log is", () => {
    const lines = testsLines({
      counted: {
        command: "npm run coverage",
        coverage_run: true,
        result: {
          exit_code: 1,
          counts: { passed: 12, failed: 1, skipped: 2 },
          coverage_pct: 81.24,
          coverage_source: "coverage/lcov.info",
        },
        duration_ms: 4200,
        log: ".ololo/probes/0003-tests.log",
        probe_seq: 3,
        inherited: false,
      },
      tests_score: 44.5,
      coverage_score: 72,
    });
    expect(lines).toEqual([
      { label: "tests", value: "12 passed · 1 failed · 2 skipped · score 45", tone: "red" },
      { label: "coverage", value: "81.2% · score 72" },
      { label: "log", value: ".ololo/probes/0003-tests.log" },
    ]);
  });

  it("names a carried-over run and a check's own run that measured nothing", () => {
    const lines = testsLines({
      counted: {
        command: "npm test",
        result: { exit_code: 0 },
        duration_ms: 900,
        probe_seq: 2,
        inherited: true,
      },
      attempt: { status: "timeout", command: "npm test", duration_ms: 300000 },
      tests_score: 100,
    });
    expect(lines[0]).toEqual({
      label: "tests",
      value: "passed (exit 0) · score 100 · from check #2",
      tone: undefined,
    });
    expect(lines[1]).toEqual({
      label: "this run",
      value: "this check's run timed out",
      tone: "amber",
    });
    const declined = testsLines({
      attempt: { status: "declined", command: "make test", duration_ms: 0 },
    });
    expect(declined).toEqual([
      { label: "tests", value: "not allowed by .ololo/settings.json (make test)", tone: "amber" },
    ]);
    expect(testsLines(null)).toEqual([]);
  });

  it("puts a run's verdict in words", () => {
    expect(testsVerdict({ counts: { passed: 3, failed: 0 } })).toBe("3 passed");
    expect(testsVerdict({ counts: { passed: 0, failed: 0 } })).toBe("no tests ran");
    expect(testsVerdict({ exit_code: 2 })).toBe("failed (exit 2)");
    expect(testsVerdict({ exit_code: null })).toBe("killed");
  });
});
