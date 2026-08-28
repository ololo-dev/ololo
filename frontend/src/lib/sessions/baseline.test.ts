import { describe, it, expect } from "vitest";
import { resolveBaseline } from "./baseline";
import type { PlayerHistoryCommit, PlayerTaskSummaryEntry } from "$lib/types/arena";

function mkCommit(over: Partial<PlayerHistoryCommit> = {}): PlayerHistoryCommit {
  return {
    sha: "sha-" + Math.random().toString(36).slice(2, 8),
    author_name: "bot",
    author_email: "bot@x",
    author_time: "2026-01-01T00:00:00Z",
    message: "",
    files: [],
    ...over,
  };
}

function mkTask(over: Partial<PlayerTaskSummaryEntry> = {}): PlayerTaskSummaryEntry {
  return {
    task_id: "task-x",
    ordinal: 1,
    title: "X",
    content: "",
    tags: [],
    adapted_content: "",
    result: null,
    scheduler_state: null,
    ...over,
  };
}

describe("resolveBaseline", () => {
  it("task N-1 has commits → parent = its last (max author_time)", () => {
    const tasks = [mkTask({ task_id: "t1", ordinal: 1 }), mkTask({ task_id: "t2", ordinal: 2 })];
    const attributed = new Map([
      [
        "t1",
        [
          mkCommit({ sha: "a", author_time: "2026-01-01T00:00:00Z" }),
          mkCommit({ sha: "b", author_time: "2026-01-02T00:00:00Z" }),
        ],
      ],
      ["t2", [mkCommit({ sha: "c" })]],
    ]);
    const r = resolveBaseline(2, tasks, attributed, null);
    expect(r.parentCommit!.sha).toBe("b");
    expect(r.commits.map((c) => c.sha)).toEqual(["c"]);
    expect(r.mode).toBe("range");
  });

  it("task N-1 empty, N-2 has commits → parent = N-2 last", () => {
    const tasks = [
      mkTask({ task_id: "t1", ordinal: 1 }),
      mkTask({ task_id: "t2", ordinal: 2 }),
      mkTask({ task_id: "t3", ordinal: 3 }),
    ];
    const attributed = new Map([
      ["t1", [mkCommit({ sha: "old" })]],
      ["t2", []],
      ["t3", [mkCommit({ sha: "cur" })]],
    ]);
    const r = resolveBaseline(3, tasks, attributed, null);
    expect(r.parentCommit!.sha).toBe("old");
    expect(r.mode).toBe("range");
  });

  it("first task with own commits → baseline is the parent, not a listed commit", () => {
    const tasks = [mkTask({ task_id: "t1", ordinal: 0 })];
    const attributed = new Map([["t1", [mkCommit({ sha: "cur" })]]]);
    const baseline = mkCommit({ sha: "base" });
    const r = resolveBaseline(0, tasks, attributed, baseline);
    expect(r.parentCommit!.sha).toBe("base");
    expect(r.commits.map((c) => c.sha)).toEqual(["cur"]);
    expect(r.mode).toBe("range");
  });

  it("later task with baseline parent still uses range mode", () => {
    const tasks = [mkTask({ task_id: "t0", ordinal: 0 }), mkTask({ task_id: "t1", ordinal: 1 })];
    const attributed = new Map([["t1", [mkCommit({ sha: "cur" })]]]);
    const baseline = mkCommit({ sha: "base" });
    const r = resolveBaseline(1, tasks, attributed, baseline);
    expect(r.parentCommit!.sha).toBe("base");
    expect(r.mode).toBe("range");
  });

  it("first task (ordinal 0) with no attributed commits → no commits to show", () => {
    const tasks = [mkTask({ task_id: "t0", ordinal: 0 })];
    const attributed = new Map();
    const baseline = mkCommit({ sha: "base" });
    const r = resolveBaseline(0, tasks, attributed, baseline);
    expect(r.parentCommit!.sha).toBe("base");
    expect(r.commits).toEqual([]);
    expect(r.mode).toBe("per-commit");
  });

  it("no prior commits, no baseline → per-commit mode", () => {
    const tasks = [mkTask({ task_id: "t1", ordinal: 1 })];
    const attributed = new Map([["t1", [mkCommit({ sha: "cur" })]]]);
    const r = resolveBaseline(1, tasks, attributed, null);
    expect(r.parentCommit).toBeNull();
    expect(r.mode).toBe("per-commit");
  });

  it("no attributed commits for task → empty commits array, per-commit mode", () => {
    const tasks = [mkTask({ task_id: "t1", ordinal: 1 })];
    const attributed = new Map();
    const r = resolveBaseline(1, tasks, attributed, null);
    expect(r.commits).toEqual([]);
    expect(r.mode).toBe("per-commit");
  });

  it("commits sorted by author_time ascending", () => {
    const tasks = [mkTask({ task_id: "t1", ordinal: 1 })];
    const attributed = new Map([
      [
        "t1",
        [
          mkCommit({ sha: "3", author_time: "2026-01-03T00:00:00Z" }),
          mkCommit({ sha: "1", author_time: "2026-01-01T00:00:00Z" }),
          mkCommit({ sha: "2", author_time: "2026-01-02T00:00:00Z" }),
        ],
      ],
    ]);
    const r = resolveBaseline(1, tasks, attributed, null);
    expect(r.commits.map((c) => c.sha)).toEqual(["1", "2", "3"]);
  });

  it("matches task by task_id from tasks[ordinal-1]", () => {
    const tasks = [mkTask({ task_id: "aaa", ordinal: 1 }), mkTask({ task_id: "bbb", ordinal: 2 })];
    const attributed = new Map([
      ["aaa", [mkCommit({ sha: "a-c" })]],
      ["bbb", [mkCommit({ sha: "b-c" })]],
    ]);
    const r = resolveBaseline(2, tasks, attributed, null);
    expect(r.parentCommit!.sha).toBe("a-c");
    expect(r.commits.map((c) => c.sha)).toEqual(["b-c"]);
  });
});
