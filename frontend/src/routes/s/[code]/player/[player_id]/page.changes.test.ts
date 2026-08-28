import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render } from "@testing-library/svelte";
import Page from "./+page.svelte";
import {
  makeSnapshot,
  makeTaskSummary,
  makeProbe,
  renderPage,
  type TestData,
} from "./page.test-helpers";

describe("Player detail page — changes", () => {
  beforeEach(() => {
    // These cover the detailed task panels. A running session now opens on
    // the slides, so ask for the full list explicitly.
    localStorage.setItem("player:task-view:player-1", "details");
    vi.clearAllMocks();
    vi.useRealTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("attributes feat(task_id) commit to matching task Changes tab", async () => {
    const { fireEvent } = await import("@testing-library/svelte");
    const history = {
      commits: [
        {
          sha: "base-sha",
          author_name: "ololo",
          author_email: "o@x",
          author_time: "2026-01-01T00:00:00Z",
          message: "ololo snapshot: session start @ 2026-01-01T00:00:00Z",
          files: [],
        },
        {
          sha: "feat-sha",
          author_name: "bot",
          author_email: "b@x",
          author_time: "2026-01-02T00:00:00Z",
          message: "feat(task-1): implement parser",
          files: [{ path: "main.rs", status: "added" as const, patch: "+fn main() {}" }],
        },
      ],
    };
    const { container, getByTestId } = renderPage({
      live: true,
      snapshot: makeSnapshot({
        probes: [makeProbe({ task_id: "task-1", task_ordinal: 1 })],
        tasks: [
          makeTaskSummary({
            task_id: "task-1",
            ordinal: 1,
            title: "Parser Task",
            scheduler_state: { state: "awaiting_result", activated_at: null, deadline_at: null },
          }),
        ],
      }),
      token: "token",
      playerName: "Test Player",
      history,
    });
    await fireEvent.click(getByTestId("tab-changes"));
    // File header visible, patch collapsed — click file to expand
    const fileBtn = container.querySelector('[data-testid="diff-view"] button');
    if (fileBtn) await fireEvent.click(fileBtn);
    expect(container.textContent).toContain("main.rs");
  });

  it("unattributed commit appears only in the page Changes tab, not the task panel", async () => {
    const { fireEvent } = await import("@testing-library/svelte");
    const history = {
      commits: [
        {
          sha: "rand-sha",
          author_name: "bot",
          author_email: "b@x",
          author_time: "2026-01-02T00:00:00Z",
          message: "random commit no convention",
          files: [{ path: "random.txt", status: "added" as const, patch: "+random" }],
        },
      ],
    };
    const { container, getByTestId } = renderPage({
      live: true,
      snapshot: makeSnapshot({
        probes: [makeProbe({ task_id: "task-1", task_ordinal: 1 })],
        tasks: [
          makeTaskSummary({
            task_id: "task-1",
            ordinal: 1,
            title: "Task One",
            scheduler_state: { state: "awaiting_result", activated_at: null, deadline_at: null },
          }),
        ],
      }),
      token: "token",
      playerName: "Test Player",
      history,
    });
    // No commits attribute to the task, so the task panel's Changes tab is absent.
    expect(container.querySelector('[data-testid="tab-changes"]')).toBeNull();
    // The unattributed commit is still reachable via the page-level Changes tab.
    await fireEvent.click(getByTestId("ptab-changes"));
    expect(container.textContent).toContain("random commit no convention");
  });

  it("empty changes → Changes tab hidden", async () => {
    const { container } = renderPage({
      live: true,
      snapshot: makeSnapshot({
        probes: [makeProbe({ task_id: "task-1", task_ordinal: 1 })],
        tasks: [
          makeTaskSummary({
            task_id: "task-1",
            ordinal: 1,
            title: "Empty Task",
            scheduler_state: { state: "awaiting_result", activated_at: null, deadline_at: null },
          }),
        ],
      }),
      token: "token",
      playerName: "Test Player",
      history: { commits: [] },
    });
    expect(container.querySelector('[data-testid="tab-changes"]')).toBeNull();
  });

  it("session-start commit is a diff base, never listed in the first task's Changes", async () => {
    const { fireEvent } = await import("@testing-library/svelte");
    const history = {
      commits: [
        {
          sha: "base-sha",
          author_name: "ololo",
          author_email: "o@x",
          author_time: "2026-01-01T00:00:00Z",
          message: "ololo snapshot: session start @ 2026-01-01T00:00:00Z",
          files: [{ path: "init.txt", status: "added" as const, patch: "+init" }],
        },
      ],
    };
    const { container, getByTestId } = renderPage({
      live: true,
      snapshot: makeSnapshot({
        probes: [makeProbe({ task_id: "task-1", task_ordinal: 0 })],
        tasks: [
          makeTaskSummary({
            task_id: "task-1",
            ordinal: 0,
            title: "First Task",
            scheduler_state: { state: "awaiting_result", activated_at: null, deadline_at: null },
          }),
        ],
      }),
      token: "token",
      playerName: "Test Player",
      history,
    });
    // The seed snapshot is the baseline, not a change of the first task —
    // no per-task Changes tab, and init.txt never lands in the task panel.
    expect(container.querySelector('[data-testid="tab-changes"]')).toBeNull();
    expect(container.textContent).not.toContain("init.txt");
    // It stays visible in the page-level Changes tab, labelled as the baseline.
    await fireEvent.click(getByTestId("ptab-changes"));
    expect(container.textContent).toContain("session start");
    expect(container.textContent).toContain("Session start");
  });
});
