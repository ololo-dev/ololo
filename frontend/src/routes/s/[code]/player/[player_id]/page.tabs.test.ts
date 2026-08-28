import { describe, it, expect, vi, beforeEach } from "vitest";
import { fireEvent } from "@testing-library/svelte";
import { makeSnapshot, makeTaskSummary, makeProbe, renderPage } from "./page.test-helpers";
import type { AgentSessionStats, TaskStatsEntry } from "$lib/types/arena";

function makeAgentStats(over: Partial<AgentSessionStats> = {}): AgentSessionStats {
  return {
    agent: "claude-code",
    agent_session_id: "sess-1",
    model: "claude-opus-5",
    input_tokens: 1_000,
    output_tokens: 500,
    cache_read_tokens: 0,
    cache_write_tokens: 0,
    reasoning_tokens: 0,
    cost: 0.5,
    user_messages: 2,
    assistant_messages: 3,
    tool_calls: 4,
    tools: { Read: 3, Edit: 1 },
    skills: { verify: 1 },
    ...over,
  };
}

function makeStats(over: Partial<TaskStatsEntry> = {}): TaskStatsEntry {
  return {
    task_id: "task-1",
    task_ordinal: 1,
    window_started_at: "2026-01-01T00:00:00Z",
    window_ended_at: "2026-01-01T00:02:00Z",
    input_tokens: 1_000,
    output_tokens: 500,
    cache_read_tokens: 0,
    cache_write_tokens: 0,
    reasoning_tokens: 0,
    cost: 0.5,
    user_messages: 2,
    assistant_messages: 3,
    tool_calls: 4,
    agents: [makeAgentStats()],
    created_at: "2026-01-01T00:02:00Z",
    updated_at: "2026-01-01T00:02:00Z",
    ...over,
  };
}

const TASKS = [
  makeTaskSummary({
    task_id: "task-1",
    ordinal: 1,
    title: "Task One",
    total_points: 10,
    scheduler_state: { state: "awaiting_result", activated_at: null, deadline_at: null },
  }),
];

describe("Player detail page — section tabs", () => {
  beforeEach(() => {
    // These cover the detailed task panels. A running session now opens on
    // the slides, so ask for the full list explicitly.
    localStorage.setItem("player:task-view:player-1", "details");
    vi.clearAllMocks();
  });

  it("defaults to the Tasks tab", () => {
    const { getByTestId } = renderPage({
      live: true,
      snapshot: makeSnapshot({
        probes: [makeProbe({ task_id: "task-1", task_ordinal: 1 })],
        tasks: TASKS,
      }),
      token: "token",
      playerName: "Test Player",
    });
    expect(getByTestId("ptab-tasks").getAttribute("aria-selected")).toBe("true");
    expect(getByTestId("task-panel-1")).toBeTruthy();
  });

  it("Judges tab lists verdicts from every task with a points total", async () => {
    const { container, getByTestId } = renderPage({
      live: true,
      snapshot: makeSnapshot({
        probes: [makeProbe({ task_id: "task-1", task_ordinal: 1 })],
        tasks: TASKS,
        judge_results: [
          {
            task_id: "task-1",
            judge_slug: "code-cleanliness",
            judge_name: "Code Cleanliness",
            rating: 8,
            feedback: "Tidy diff, good naming.",
            point_delta: 6,
            created_at: "2026-01-01T00:03:00Z",
          },
          {
            task_id: "task-1",
            judge_slug: "test-coverage",
            judge_name: "Test Coverage",
            rating: 3,
            feedback: "No tests added.",
            point_delta: -4,
            created_at: "2026-01-01T00:03:30Z",
          },
        ],
      }),
      token: "token",
      playerName: "Test Player",
    });

    await fireEvent.click(getByTestId("ptab-judges"));
    expect(getByTestId("player-judges-tab")).toBeTruthy();
    expect(container.textContent).toContain("Tidy diff, good naming.");
    expect(container.textContent).toContain("No tests added.");
    // 6 + (-4) = +2 across both judges.
    expect(getByTestId("judges-total-points").textContent?.trim()).toBe("+2");
  });

  it("Judges tab shows an empty state when no judge has run", async () => {
    const { container, getByTestId } = renderPage({
      live: true,
      snapshot: makeSnapshot({
        probes: [makeProbe({ task_id: "task-1", task_ordinal: 1 })],
        tasks: TASKS,
      }),
      token: "token",
      playerName: "Test Player",
    });
    await fireEvent.click(getByTestId("ptab-judges"));
    expect(container.textContent).toContain("No judge verdicts yet");
  });

  it("Stats tab totals across tasks and shows a per-task row", async () => {
    const { container, getByTestId } = renderPage({
      live: true,
      snapshot: makeSnapshot({
        probes: [makeProbe({ task_id: "task-1", task_ordinal: 1 })],
        tasks: TASKS,
      }),
      token: "token",
      playerName: "Test Player",
      // With prompt caching almost all of the input is a cache read; the
      // headline "in" must be what the model was actually fed, not the
      // uncached sliver (a real session showed "82 in" against 2.2M read).
      taskStats: { entries: [makeStats({ cache_read_tokens: 9_000 })] },
    });

    await fireEvent.click(getByTestId("ptab-stats"));
    expect(getByTestId("player-stats-tab")).toBeTruthy();
    expect(getByTestId("stats-total-tokens").textContent).toContain("10.0k / 500");
    expect(getByTestId("stats-row-1").textContent).toContain("Task One");
    // Rolled-up tool usage from the agent session.
    expect(container.textContent).toContain("Read");
  });

  it("Stats tab shows an empty state when no stats were reported", async () => {
    const { container, getByTestId } = renderPage({
      live: true,
      snapshot: makeSnapshot({
        probes: [makeProbe({ task_id: "task-1", task_ordinal: 1 })],
        tasks: TASKS,
      }),
      token: "token",
      playerName: "Test Player",
    });
    await fireEvent.click(getByTestId("ptab-stats"));
    expect(container.textContent).toContain("No implementation statistics reported yet");
  });

  it("Changes tab lists every commit, labelled by task", async () => {
    const { container, getByTestId } = renderPage({
      live: true,
      snapshot: makeSnapshot({
        probes: [makeProbe({ task_id: "task-1", task_ordinal: 1 })],
        tasks: TASKS,
      }),
      token: "token",
      playerName: "Test Player",
      history: {
        commits: [
          {
            sha: "feat-sha",
            author_name: "bot",
            author_email: "b@x",
            author_time: "2026-01-02T00:00:00Z",
            message: "feat(task-1): implement parser",
            files: [{ path: "main.rs", status: "added", patch: "+fn main() {}" }],
          },
        ],
      },
    });

    await fireEvent.click(getByTestId("ptab-changes"));
    expect(container.textContent).toContain("implement parser");
    expect(container.textContent).toContain("Task #1");
  });

  it("arrow keys move between section tabs", async () => {
    const { getByTestId } = renderPage({
      live: true,
      snapshot: makeSnapshot({
        probes: [makeProbe({ task_id: "task-1", task_ordinal: 1 })],
        tasks: TASKS,
      }),
      token: "token",
      playerName: "Test Player",
    });
    const strip = getByTestId("ptab-tasks").parentElement!;
    await fireEvent.keyDown(strip, { key: "ArrowRight" });
    expect(getByTestId("ptab-changes").getAttribute("aria-selected")).toBe("true");
    await fireEvent.keyDown(strip, { key: "ArrowLeft" });
    expect(getByTestId("ptab-tasks").getAttribute("aria-selected")).toBe("true");
  });
});
