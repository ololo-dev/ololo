import { describe, it, expect } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import TaskChanges from "./TaskChanges.svelte";
import type { PlayerHistoryCommit } from "$lib/types/arena";

function mkCommit(
  files: { path: string; status: "added" | "modified" | "deleted"; patch: string }[],
  time = "2026-01-01T00:00:00Z",
  msg = "feat(t): x",
): PlayerHistoryCommit {
  return {
    sha: "sha-" + Math.random().toString(36).slice(2, 8),
    author_name: "bot",
    author_email: "bot@x",
    author_time: time,
    message: msg,
    files,
  };
}

describe("TaskChanges", () => {
  it("range mode renders file list, expand file to see diff", async () => {
    const commits = [
      mkCommit([{ path: "a.txt", status: "added", patch: "+a" }], "2026-01-01T00:00:00Z"),
      mkCommit(
        [{ path: "a.txt", status: "modified", patch: "@@ -1 +1 @@\n-a\n+aa" }],
        "2026-01-02T00:00:00Z",
      ),
    ];
    const { container, getByText } = render(TaskChanges, {
      props: { commits, mode: "range", testid: "task-changes-1" },
    });
    expect(container.querySelector('[data-testid="task-changes-1"]')).toBeTruthy();
    // File header visible (collapsed), patch hidden
    expect(getByText("a.txt")).toBeTruthy();
    expect(container.textContent).not.toContain("+a");
    // Click file to expand
    await fireEvent.click(getByText("a.txt"));
    expect(container.textContent).toContain("+a");
  });

  it("per-commit mode renders each commit with file list", async () => {
    const commits = [
      mkCommit([{ path: "a.txt", status: "added", patch: "+a" }]),
      mkCommit([{ path: "b.txt", status: "added", patch: "+b" }]),
    ];
    const { container } = render(TaskChanges, {
      props: { commits, mode: "per-commit", testid: "task-changes-2" },
    });
    const diffViews = container.querySelectorAll('[data-testid="diff-view"]');
    expect(diffViews.length).toBe(2);
  });

  it("empty commits → empty-state text", () => {
    const { container, getByText } = render(TaskChanges, {
      props: { commits: [], mode: "per-commit", testid: "task-changes-3" },
    });
    expect(getByText("No changes attributed to this task.")).toBeTruthy();
    expect(container.querySelector('[data-testid="diff-view"]')).toBeNull();
  });

  it("has data-testid on root", () => {
    const { container } = render(TaskChanges, {
      props: { commits: [], mode: "per-commit", testid: "tc-root" },
    });
    expect(container.querySelector('[data-testid="tc-root"]')).toBeTruthy();
  });
});
