import { describe, it, expect } from "vitest";
import { attributeCommits } from "./attribution";
import type { PlayerHistoryCommit } from "$lib/types/arena";

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

describe("attributeCommits", () => {
  it("matches feat(task-id): description", () => {
    const c = mkCommit({ message: "feat(my-task): does thing", sha: "a" });
    const r = attributeCommits([c]);
    expect(r.attributed.get("my-task")).toBeDefined();
    expect(r.attributed.get("my-task")!.length).toBe(1);
    expect(r.attributed.get("my-task")![0].sha).toBe("a");
    expect(r.unattributed).toHaveLength(0);
    expect(r.baseline).toBeNull();
  });

  it("matches feat(task-with-hyphens-1): x — id with hyphens and digits", () => {
    const c = mkCommit({ message: "feat(task-with-hyphens-1): x", sha: "b" });
    const r = attributeCommits([c]);
    expect(r.attributed.get("task-with-hyphens-1")).toBeDefined();
  });

  it("tags ololo snapshot: session start @ as baseline, excludes from attribution", () => {
    const c = mkCommit({
      message: "ololo snapshot: session start @ 2026-01-01T00:00:00Z",
      sha: "base",
    });
    const r = attributeCommits([c]);
    expect(r.baseline).not.toBeNull();
    expect(r.baseline!.sha).toBe("base");
    expect(r.attributed.size).toBe(0);
    expect(r.unattributed).toHaveLength(0);
  });

  it("matches session start BEFORE feat regex — feat(...) in session-start message is baseline", () => {
    const c = mkCommit({ message: "ololo snapshot: session start @ feat(x): y", sha: "base" });
    const r = attributeCommits([c]);
    expect(r.baseline).not.toBeNull();
    expect(r.attributed.size).toBe(0);
  });

  it("attributes task-addressed auxiliary commits (wip/artifact/flag/memory)", () => {
    const result = attributeCommits([
      mkCommit({ message: "feat(t1): impl", sha: "f" }),
      mkCommit({ message: "wip(t1): checkpoint", sha: "w" }),
      mkCommit({ message: "artifact(t1): sync", sha: "a" }),
      mkCommit({ message: "flag(t1): done.md", sha: "g" }),
      mkCommit({ message: "memory(t1): sources @ 2026", sha: "m" }),
      mkCommit({ message: "artifact: sync", sha: "legacy" }),
    ]);
    expect(
      result.attributed
        .get("t1")
        ?.map((c) => c.sha)
        .sort(),
    ).toEqual(["a", "f", "g", "m", "w"]);
    // The legacy un-addressed form stays unattributed.
    expect(result.unattributed.map((c) => c.sha)).toEqual(["legacy"]);
  });

  it("tags fix: no scope as unattributed", () => {
    const c = mkCommit({ message: "fix: no scope", sha: "u" });
    const r = attributeCommits([c]);
    expect(r.unattributed).toHaveLength(1);
    expect(r.unattributed[0].sha).toBe("u");
    expect(r.attributed.size).toBe(0);
  });

  it("tags feat(): empty scope as unattributed", () => {
    const c = mkCommit({ message: "feat(): empty scope", sha: "u2" });
    const r = attributeCommits([c]);
    expect(r.unattributed).toHaveLength(1);
  });

  it("handles leading/trailing whitespace in message", () => {
    const c = mkCommit({ message: "  feat(my-task): padded  ", sha: "p" });
    const r = attributeCommits([c]);
    expect(r.attributed.get("my-task")).toBeDefined();
  });

  it("empty message → unattributed", () => {
    const c = mkCommit({ message: "", sha: "e" });
    const r = attributeCommits([c]);
    expect(r.unattributed).toHaveLength(1);
  });

  it("sorts attributed commits by author_time ascending within each task", () => {
    const commits = [
      mkCommit({ message: "feat(t): third", sha: "3", author_time: "2026-01-03T00:00:00Z" }),
      mkCommit({ message: "feat(t): first", sha: "1", author_time: "2026-01-01T00:00:00Z" }),
      mkCommit({ message: "feat(t): second", sha: "2", author_time: "2026-01-02T00:00:00Z" }),
    ];
    const r = attributeCommits(commits);
    const arr = r.attributed.get("t")!;
    expect(arr.map((c) => c.sha)).toEqual(["1", "2", "3"]);
  });

  it("groups multiple tasks independently", () => {
    const commits = [
      mkCommit({ message: "feat(a): x", sha: "a1" }),
      mkCommit({ message: "feat(b): y", sha: "b1" }),
      mkCommit({ message: "feat(a): z", sha: "a2", author_time: "2026-01-02T00:00:00Z" }),
      mkCommit({ message: "feat(a): w", sha: "a0", author_time: "2025-12-31T00:00:00Z" }),
    ];
    const r = attributeCommits(commits);
    expect(r.attributed.get("a")!.map((c) => c.sha)).toEqual(["a0", "a1", "a2"]);
    expect(r.attributed.get("b")!.map((c) => c.sha)).toEqual(["b1"]);
  });

  it("mixed: baseline + attributed + unattributed", () => {
    const commits = [
      mkCommit({ message: "feat(t1): impl", sha: "t1c" }),
      mkCommit({ message: "ololo snapshot: session start @ 2026", sha: "base" }),
      mkCommit({ message: "random commit", sha: "rand" }),
    ];
    const r = attributeCommits(commits);
    expect(r.baseline!.sha).toBe("base");
    expect(r.attributed.get("t1")!.length).toBe(1);
    expect(r.unattributed.map((c) => c.sha)).toEqual(["rand"]);
  });

  describe("task windows (legacy commits without a task-id prefix)", () => {
    const windows = [
      { task_id: "t0", ordinal: 0, started_at: "2026-01-01T10:00:00Z" },
      { task_id: "t1", ordinal: 1, started_at: "2026-01-01T11:00:00Z" },
    ];

    it("assigns legacy commits to the task whose window contains them", () => {
      const commits = [
        mkCommit({ message: "flag: done.md", sha: "f0", author_time: "2026-01-01T10:30:00Z" }),
        mkCommit({ message: "artifact: sync", sha: "a1", author_time: "2026-01-01T11:30:00Z" }),
      ];
      const r = attributeCommits(commits, windows);
      expect(r.attributed.get("t0")!.map((c) => c.sha)).toEqual(["f0"]);
      expect(r.attributed.get("t1")!.map((c) => c.sha)).toEqual(["a1"]);
      expect(r.unattributed).toHaveLength(0);
    });

    it("the last window is open-ended", () => {
      const late = mkCommit({
        message: "ololo snapshot: final @ 2026",
        sha: "fin",
        author_time: "2026-01-01T12:00:00Z",
      });
      const r = attributeCommits([late], windows);
      expect(r.attributed.get("t1")!.map((c) => c.sha)).toEqual(["fin"]);
    });

    it("commits before every window fold into the first task", () => {
      const early = mkCommit({
        message: "fix: skewed clock",
        sha: "e",
        author_time: "2026-01-01T09:59:00Z",
      });
      const r = attributeCommits([early], windows);
      expect(r.attributed.get("t0")!.map((c) => c.sha)).toEqual(["e"]);
      expect(r.unattributed).toHaveLength(0);
    });

    it("compares epochs across offsets — +02:00 commit lands in the UTC window", () => {
      // 12:30+02:00 = 10:30Z → t0's window, not t1's.
      const c = mkCommit({
        message: "artifact: sync",
        sha: "tz",
        author_time: "2026-01-01T12:30:00+02:00",
      });
      const r = attributeCommits([c], windows);
      expect(r.attributed.get("t0")!.map((x) => x.sha)).toEqual(["tz"]);
    });

    it("explicit task-id prefixes still win over windows", () => {
      const c = mkCommit({
        message: "artifact(t0): sync",
        sha: "x",
        author_time: "2026-01-01T11:30:00Z",
      });
      const r = attributeCommits([c], windows);
      expect(r.attributed.get("t0")!.map((x) => x.sha)).toEqual(["x"]);
      expect(r.attributed.get("t1")).toBeUndefined();
    });

    it("tasks without started_at open no window", () => {
      const c = mkCommit({ message: "random", sha: "r", author_time: "2026-01-01T10:30:00Z" });
      const r = attributeCommits([c], [{ task_id: "t", ordinal: 0, started_at: null }]);
      expect(r.unattributed.map((x) => x.sha)).toEqual(["r"]);
    });

    it("baseline never falls into a window", () => {
      const base = mkCommit({
        message: "ololo snapshot: session start @ 2026",
        sha: "base",
        author_time: "2026-01-01T10:30:00Z",
      });
      const r = attributeCommits([base], windows);
      expect(r.baseline!.sha).toBe("base");
      expect(r.attributed.size).toBe(0);
    });
  });
});
