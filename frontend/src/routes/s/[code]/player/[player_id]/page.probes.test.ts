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

describe("Player detail page — probes", () => {
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

  it("renders task panel with probes shown newest on top when expanded", () => {
    const probes = [
      makeProbe({
        id: "probe-1",
        task_id: "task-1",
        task_title: "First Task",
        task_ordinal: 1,
        test_command: "check true",
        attempt: 1,
        rendered_command: "check true",
        state: "resolved",
        outcome: "failed",
        dispatched_at: "2024-01-01T00:00:00Z",
        deadline_at: "2024-01-01T00:00:30Z",
        resolved_at: "2024-01-01T00:00:15Z",
        point_delta: -5,
        result: { status: "failed", expected: "true", actual: "false" },
      }),
      makeProbe({
        id: "probe-2",
        task_id: "task-1",
        task_title: "First Task",
        task_ordinal: 1,
        test_command: "check true again",
        attempt: 2,
        rendered_command: "check true again",
        state: "resolved",
        outcome: "passed",
        dispatched_at: "2024-01-01T00:00:30Z",
        deadline_at: "2024-01-01T00:01:00Z",
        resolved_at: "2024-01-01T00:00:45Z",
        point_delta: 10,
        result: { status: "passed", expected: "true", actual: "true" },
      }),
    ];
    const { container } = renderPage({
      live: true,
      snapshot: makeSnapshot({
        probes,
        tasks: [
          makeTaskSummary({
            task_id: "task-1",
            title: "First Task",
            ordinal: 1,
            scheduler_state: { state: "awaiting_result", activated_at: null, deadline_at: null },
          }),
        ],
      }),
      token: "token",
      playerName: "Test Player",
    });

    const text = container.textContent ?? "";
    expect(text).toContain("First Task");
    expect(text).toContain("Failed");
    expect(text).toContain("Passed");
  });

  it("renders probe expected and actual results when probe expanded", async () => {
    const { fireEvent } = await import("@testing-library/svelte");
    const probes = [
      makeProbe({
        id: "probe-1",
        task_id: "task-current",
        task_title: "Current task",
        task_ordinal: 2,
        test_command: "check true",
        attempt: 1,
        rendered_command: "check true",
        state: "resolved",
        outcome: "failed",
        dispatched_at: "2024-01-01T00:00:00Z",
        deadline_at: "2024-01-01T00:00:30Z",
        resolved_at: "2024-01-01T00:00:15Z",
        point_delta: -5,
        result: { status: "failed", expected: "true", actual: "false" },
      }),
    ];
    const { container, getByTestId } = renderPage({
      live: true,
      snapshot: makeSnapshot({
        probes,
        tasks: [
          makeTaskSummary({
            task_id: "task-current",
            ordinal: 2,
            title: "Current task",
            scheduler_state: { state: "awaiting_result", activated_at: null, deadline_at: null },
          }),
        ],
      }),
      token: "token",
      playerName: "Test Player",
    });
    const probeRow = getByTestId("probe-row-probe-1");
    // Latest probe auto-expanded — no click needed
    const text = container.textContent ?? "";
    expect(text).toContain("Expected");
    expect(text).toContain("true");
    expect(text).toContain("Actual");
    expect(text).toContain("false");
  });

  it("renders section chips (not tags) in task panel header", () => {
    const { container } = renderPage({
      live: true,
      snapshot: makeSnapshot({
        probes: [
          makeProbe({
            id: "probe-1",
            task_id: "task-1",
            task_title: "Tagged Task",
            task_ordinal: 1,
            test_command: "run test",
            attempt: 1,
            rendered_command: "run test",
            state: "dispatched",
            dispatched_at: "2024-01-01T00:00:00Z",
            deadline_at: "2024-01-01T00:00:30Z",
            resolved_at: null,
            point_delta: 0,
            result: null,
          }),
        ],
        tasks: [
          makeTaskSummary({
            title: "Tagged Task",
            content: "Implement parser for inputs",
            tags: ["rust", "parser"],
            scheduler_state: { state: "awaiting_result", activated_at: null, deadline_at: null },
          }),
        ],
      }),
      token: "token",
      playerName: "Test Player",
    });

    expect(container.textContent).toContain("Tagged Task");
    // Tags no longer render in the header; section chips do.
    expect(container.textContent).not.toContain("rust");
    expect(container.querySelector('[data-testid^="task-sections-"]')?.textContent).toContain(
      "probes (1)",
    );
  });

  it("shows pending badge for unresponded probes", () => {
    const { container } = renderPage({
      live: true,
      snapshot: makeSnapshot({
        probes: [
          makeProbe({
            id: "probe-pending",
            task_id: "task-1",
            task_title: "Pending Task",
            task_ordinal: 1,
            test_command: "check file",
            attempt: 1,
            rendered_command: "check file",
            state: "dispatched",
            dispatched_at: "2024-01-01T00:00:00Z",
            deadline_at: "2024-01-01T00:00:30Z",
            resolved_at: null,
            point_delta: 5,
            result: null,
          }),
        ],
        tasks: [
          makeTaskSummary({
            task_id: "task-1",
            ordinal: 1,
            title: "Pending Task",
            scheduler_state: { state: "awaiting_result", activated_at: null, deadline_at: null },
          }),
        ],
      }),
      token: "token",
      playerName: "Test Player",
    });

    expect(container.textContent).toContain("pending");
    expect(container.textContent).toContain("+5");
  });

  it("shows Tasks widget with completed/total", () => {
    const { container } = renderPage({
      live: true,
      snapshot: makeSnapshot({
        probes: [makeProbe()],
        tasks: [
          makeTaskSummary({
            task_id: "task-done",
            ordinal: 1,
            result: {
              status: "completed",
              submitted_answer: null,
              correct_answer: null,
              score_delta: 10,
              evaluated_at: null,
            },
          }),
          makeTaskSummary({
            task_id: "task-current",
            ordinal: 2,
            scheduler_state: { state: "awaiting_result", activated_at: null, deadline_at: null },
          }),
        ],
        total_tasks: 5,
      }),
      token: "token",
      playerName: "Test Player",
    });

    const text = container.textContent ?? "";
    expect(text).toContain("Tasks");
    expect(text).toContain("1/5");
  });

  it("shows Tasks widget 0/total when no current task", () => {
    const { container } = renderPage({
      live: true,
      snapshot: makeSnapshot({
        tasks: [],
        total_tasks: 3,
      }),
      token: "token",
      playerName: "Test Player",
    });

    const text = container.textContent ?? "";
    expect(text).toContain("0/3");
  });
});
