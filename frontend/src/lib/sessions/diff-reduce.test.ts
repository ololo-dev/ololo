import { describe, it, expect } from "vitest";
import { reduceDiff } from "./diff-reduce";
import type { PlayerHistoryCommit, PlayerHistoryFileDiff } from "$lib/types/arena";

function mkFile(
  path: string,
  status: "added" | "modified" | "deleted",
  patch: string,
): PlayerHistoryFileDiff {
  return { path, status, patch };
}

function mkCommit(
  files: PlayerHistoryFileDiff[],
  time = "2026-01-01T00:00:00Z",
): PlayerHistoryCommit {
  return {
    sha: "sha-" + Math.random().toString(36).slice(2, 8),
    author_name: "bot",
    author_email: "bot@x",
    author_time: time,
    message: "feat(t): x",
    files,
  };
}

describe("reduceDiff", () => {
  it("single commit single file → passthrough", () => {
    const commits = [mkCommit([mkFile("a.txt", "added", "+a")])];
    const out = reduceDiff(commits);
    expect(out).toHaveLength(1);
    expect(out[0].path).toBe("a.txt");
    expect(out[0].status).toBe("added");
    expect(out[0].patch).toBe("+a");
  });

  it("two commits different files → both files present", () => {
    const commits = [
      mkCommit([mkFile("a.txt", "added", "+a")], "2026-01-01T00:00:00Z"),
      mkCommit([mkFile("b.txt", "added", "+b")], "2026-01-02T00:00:00Z"),
    ];
    const out = reduceDiff(commits);
    expect(out.map((f) => f.path).sort()).toEqual(["a.txt", "b.txt"]);
  });

  it("two commits same file modified → concatenated patches, status modified", () => {
    const commits = [
      mkCommit([mkFile("a.txt", "modified", "@@ -1 +1 @@\n-x\n+a")], "2026-01-01T00:00:00Z"),
      mkCommit([mkFile("a.txt", "modified", "@@ -2 +2 @@\n-y\n+bb")], "2026-01-02T00:00:00Z"),
    ];
    const out = reduceDiff(commits);
    expect(out).toHaveLength(1);
    expect(out[0].status).toBe("modified");
    expect(out[0].patch).toContain("@@ -1 +1 @@");
    expect(out[0].patch).toContain("@@ -2 +2 @@");
  });

  it("commit add then commit delete same file → status delete, add hunks dropped", () => {
    const commits = [
      mkCommit([mkFile("a.txt", "added", "+all content")], "2026-01-01T00:00:00Z"),
      mkCommit([mkFile("a.txt", "deleted", "-all content")], "2026-01-02T00:00:00Z"),
    ];
    const out = reduceDiff(commits);
    expect(out).toHaveLength(1);
    expect(out[0].status).toBe("deleted");
    expect(out[0].patch).toBe("-all content");
  });

  it("commit modified then delete → status delete, modified hunks dropped", () => {
    const commits = [
      mkCommit([mkFile("a.txt", "modified", "@@ -1 +1 @@\n-x\n+a")], "2026-01-01T00:00:00Z"),
      mkCommit([mkFile("a.txt", "deleted", "-a\n-other")], "2026-01-02T00:00:00Z"),
    ];
    const out = reduceDiff(commits);
    expect(out).toHaveLength(1);
    expect(out[0].status).toBe("deleted");
    expect(out[0].patch).toBe("-a\n-other");
  });

  it("orders patches by author_time ascending", () => {
    const commits = [
      mkCommit([mkFile("a.txt", "modified", "@@ -2 +2 @@\n-second")], "2026-01-02T00:00:00Z"),
      mkCommit([mkFile("a.txt", "modified", "@@ -1 +1 @@\n-first")], "2026-01-01T00:00:00Z"),
    ];
    const out = reduceDiff(commits);
    expect(out[0].patch).toContain("@@ -1 +1 @@");
    expect(out[0].patch).toContain("@@ -2 +2 @@");
    const idx1 = out[0].patch.indexOf("@@ -1 +1 @@");
    const idx2 = out[0].patch.indexOf("@@ -2 +2 @@");
    expect(idx1).toBeLessThan(idx2);
  });

  it("empty commits → empty result", () => {
    expect(reduceDiff([])).toEqual([]);
  });
});
