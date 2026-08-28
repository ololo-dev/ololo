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

describe("Player detail page — expand", () => {
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

  it("shows passed task collapsed and failed task expanded", () => {
    const { container } = renderPage({
      live: true,
      snapshot: makeSnapshot({
        probes: [
          makeProbe({
            id: "p1",
            task_id: "t1",
            task_ordinal: 1,
            task_title: "Alpha",
            outcome: "pass",
            state: "resolved",
            rendered_command: "echo alpha-cmd",
            result: { status: "passed", expected: null, actual: null },
            resolved_at: "2024-01-01T00:00:00Z",
            point_delta: 10,
          }),
          makeProbe({
            id: "p2",
            task_id: "t2",
            task_ordinal: 2,
            task_title: "Beta",
            outcome: "error",
            state: "resolved",
            rendered_command: "echo beta-cmd",
            result: { status: "failed", expected: null, actual: null },
            resolved_at: "2024-01-01T00:00:00Z",
            point_delta: -5,
          }),
        ],
        tasks: [
          makeTaskSummary({
            task_id: "t1",
            ordinal: 1,
            title: "Alpha",
            result: {
              status: "completed",
              submitted_answer: "a",
              correct_answer: null,
              score_delta: 10,
              evaluated_at: "2024-01-01T00:00:00Z",
            },
          }),
          makeTaskSummary({
            task_id: "t2",
            ordinal: 2,
            title: "Beta",
            scheduler_state: { state: "awaiting_result", activated_at: null, deadline_at: null },
            result: {
              status: "failed",
              submitted_answer: "b",
              correct_answer: null,
              score_delta: -5,
              evaluated_at: "2024-01-01T00:00:00Z",
            },
          }),
        ],
        total_tasks: 2,
      }),
      token: "token",
      playerName: "Test Player",
    });
    const text = container.textContent ?? "";
    expect(text).toContain("Alpha");
    expect(text).not.toContain("alpha-cmd");
    expect(text).toContain("Beta");
    // Probe rows are keyed by id; the label no longer carries "#1" now that
    // a lone first attempt is not numbered.
    expect(container.querySelector('[data-testid^="probe-row-"]')).not.toBeNull();
  });

  it("collapses probes for passed task — command hidden", () => {
    const { container } = renderPage({
      live: true,
      snapshot: makeSnapshot({
        probes: [
          makeProbe({
            id: "p1",
            task_id: "t1",
            task_ordinal: 1,
            task_title: "Passed Task",
            outcome: "pass",
            state: "resolved",
            rendered_command: "echo secret-command",
            result: { status: "passed", expected: null, actual: null },
            resolved_at: "2024-01-01T00:00:00Z",
            point_delta: 10,
          }),
        ],
        tasks: [
          makeTaskSummary({
            task_id: "t1",
            ordinal: 1,
            title: "Passed Task",
            result: {
              status: "completed",
              submitted_answer: "a",
              correct_answer: null,
              score_delta: 10,
              evaluated_at: "2024-01-01T00:00:00Z",
            },
          }),
        ],
        total_tasks: 1,
      }),
      token: "token",
      playerName: "Test Player",
    });
    const text = container.textContent ?? "";
    expect(text).toContain("Passed Task");
    expect(text).toContain("completed");
    expect(text).not.toContain("secret-command");
  });

  it("expands collapsed passed task on click to reveal probe rows", async () => {
    const { fireEvent } = await import("@testing-library/svelte");
    const rendered = renderPage({
      live: true,
      snapshot: makeSnapshot({
        probes: [
          makeProbe({
            id: "p1",
            task_id: "t1",
            task_ordinal: 1,
            attempt: 1,
            task_title: "Expandable Task",
            outcome: "pass",
            state: "resolved",
            rendered_command: "echo reveal-me",
            result: { status: "passed", expected: null, actual: null },
            resolved_at: "2024-01-01T00:00:00Z",
            point_delta: 10,
          }),
        ],
        tasks: [
          makeTaskSummary({
            task_id: "t1",
            ordinal: 1,
            title: "Expandable Task",
            result: {
              status: "completed",
              submitted_answer: "a",
              correct_answer: null,
              score_delta: 10,
              evaluated_at: "2024-01-01T00:00:00Z",
            },
          }),
        ],
        total_tasks: 1,
      }),
      token: "token",
      playerName: "Test Player",
    });
    const { container, getByText } = rendered;
    expect(container.querySelector('[data-testid^="probe-row-"]')).toBeNull();
    const header = getByText("Expandable Task");
    await fireEvent.click(header.closest("button")!);
    expect(container.querySelector('[data-testid^="probe-row-"]')).not.toBeNull();
  });

  it("hides future tasks (ordinal > current_ordinal) — no metadata leakage", () => {
    const { container } = renderPage({
      live: true,
      snapshot: makeSnapshot({
        probes: [makeProbe({ task_id: "t1", task_ordinal: 1 })],
        tasks: [
          makeTaskSummary({
            task_id: "t1",
            ordinal: 1,
            title: "Visible Task",
            scheduler_state: { state: "awaiting_result", activated_at: null, deadline_at: null },
          }),
          makeTaskSummary({
            task_id: "t2",
            ordinal: 2,
            title: "Future Secret Task",
            tags: ["secret"],
          }),
        ],
        total_tasks: 2,
      }),
      token: "token",
      playerName: "Test Player",
    });
    const text = container.textContent ?? "";
    expect(text).toContain("Visible Task");
    expect(text).not.toContain("Future Secret Task");
    expect(text).not.toContain("secret");
  });
});
