import { describe, it, expect } from "vitest";
import { render } from "@testing-library/svelte";
import PlayerTaskChat from "./PlayerTaskChat.svelte";
import type { PlayerTaskSummaryEntry, PlayerProbeEntry } from "$lib/types/arena";

function mkTask(over: Partial<PlayerTaskSummaryEntry> = {}): PlayerTaskSummaryEntry {
  return {
    task_id: "t1",
    ordinal: 1,
    title: "Task One",
    content: "do thing",
    tags: [],
    adapted_content: "",
    result: null,
    scheduler_state: null,
    ...over,
  };
}

function mkProbe(over: Partial<PlayerProbeEntry> = {}): PlayerProbeEntry {
  return {
    id: "p1",
    task_id: "t1",
    task_title: "Task One",
    task_ordinal: 1,
    adapted_test_id: "at1",
    test_command: "run",
    attempt: 1,
    rendered_command: "echo hi",
    fixture_values: null,
    expected_answer: null,
    state: "resolved",
    outcome: "pass",
    actual: "hi",
    expected: "hi",
    exit_code: 0,
    duration_ms: 12,
    dispatched_at: "2026-01-01T00:00:00Z",
    deadline_at: null,
    resolved_at: "2026-01-01T00:00:01Z",
    result: { status: "passed", expected: "hi", actual: "hi" },
    point_delta: 0,
    updated_at: null,
    ...over,
  };
}

// The completion-contract probe: its command names the done file, which is
// what marks the task open-ended for the chat.
function doneFlagProbe(over: Partial<PlayerProbeEntry> = {}): PlayerProbeEntry {
  return mkProbe({
    id: "p-done",
    adapted_test_id: "at-done",
    test_command: "cat .ololo/task-done.md",
    rendered_command: "cat .ololo/task-done.md",
    ...over,
  });
}

function judgeCheckProbe(over: Partial<PlayerProbeEntry> = {}): PlayerProbeEntry {
  return mkProbe({
    id: "p-check",
    adapted_test_id: "at-check",
    label: "registered: test-quality",
    test_command: "node --test app.test.js",
    rendered_command: "node --test app.test.js",
    outcome: "error",
    actual: "Could not find 'app.test.js'",
    expected: null,
    result: { status: "failed", expected: null, actual: "Could not find 'app.test.js'" },
    ...over,
  });
}

const completedResult = {
  status: "completed",
  submitted_answer: null,
  correct_answer: null,
  score_delta: 0,
  evaluated_at: "2026-01-01T00:01:00Z",
};

function renderChat(tasks: PlayerTaskSummaryEntry[], probes: Map<string, PlayerProbeEntry[]>) {
  return render(PlayerTaskChat, {
    props: {
      tasks,
      probesByTask: probes,
      judgeResultsByTask: new Map(),
      judgeStatusesByTask: new Map(),
      evaluationsByTask: new Map(),
      changesByTask: new Map(),
      sessionCode: "ABC123",
      playerId: "player-1",
      playerName: "Andrey",
    },
  });
}

describe("PlayerTaskChat judge-phase status", () => {
  it("renders a failing judge-registered check instead of suppressing it", () => {
    const { queryByTestId, getByTestId } = renderChat(
      [mkTask({ result: completedResult })],
      new Map([["t1", [doneFlagProbe(), judgeCheckProbe()]]]),
    );
    const bubble = getByTestId("chat-check-at-check");
    expect(bubble.textContent).toContain("Test Quality judge requested this extra check");
    expect(bubble.textContent).toContain("Could not find 'app.test.js'");
    // A plain (non-judge) failing check with no points stays quiet.
    expect(queryByTestId("chat-check-at-plain-fail")).toBeNull();
  });

  it("keeps suppressing plain zero-point failing checks", () => {
    const { queryByTestId } = renderChat(
      [mkTask({ result: completedResult })],
      new Map([
        [
          "t1",
          [
            doneFlagProbe(),
            mkProbe({
              id: "p-plain",
              adapted_test_id: "at-plain-fail",
              outcome: "error",
              actual: "not-done: file missing",
              result: { status: "failed", expected: null, actual: "not-done: file missing" },
            }),
          ],
        ],
      ]),
    );
    expect(queryByTestId("chat-check-at-plain-fail")).toBeNull();
  });

  it("shows the 'checks' banner when verdicts are in but a judge check still holds", () => {
    const { getByTestId } = renderChat(
      [mkTask({ result: completedResult })],
      new Map([["t1", [doneFlagProbe(), judgeCheckProbe()]]]),
    );
    const banner = getByTestId("chat-completion-1");
    expect(banner.textContent).toContain("Verdicts are in");
    expect(banner.textContent).toContain("an extra check");
    expect(banner.textContent).toContain("The next task starts once everything settles.");
  });

  it("shows the 'wrapping' banner when everything settled and the scheduler still holds", () => {
    const { getByTestId } = renderChat(
      [
        mkTask({
          result: completedResult,
          scheduler_state: { state: "judging", activated_at: null, deadline_at: null },
        }),
      ],
      new Map([
        [
          "t1",
          [
            doneFlagProbe(),
            judgeCheckProbe({
              outcome: "pass",
              result: { status: "passed", expected: null, actual: "ok" },
            }),
          ],
        ],
      ]),
    );
    const banner = getByTestId("chat-completion-1");
    expect(banner.textContent).toContain("All verdicts are in");
    expect(banner.textContent).toContain("the next one is moments away");
  });

  it("treats a 'judging' scheduler state as acceptance even without a result", () => {
    const { getByTestId } = renderChat(
      [
        mkTask({
          result: null,
          scheduler_state: { state: "judging", activated_at: null, deadline_at: null },
        }),
      ],
      new Map([["t1", [doneFlagProbe(), judgeCheckProbe()]]]),
    );
    expect(getByTestId("chat-completion-1").textContent).toContain("Verdicts are in");
  });

  it("drops the banner for a finished task once the next brief has landed", () => {
    const { queryByTestId } = renderChat(
      [
        mkTask({ result: completedResult }),
        mkTask({
          task_id: "t2",
          ordinal: 2,
          title: "Task Two",
          scheduler_state: { state: "active", activated_at: null, deadline_at: null },
        }),
      ],
      new Map([
        ["t1", [doneFlagProbe(), judgeCheckProbe()]],
        [
          "t2",
          [
            doneFlagProbe({
              id: "p-done-2",
              adapted_test_id: "at-done-2",
              task_id: "t2",
              task_ordinal: 2,
              state: "dispatched",
              outcome: null,
              result: null,
              resolved_at: null,
            }),
          ],
        ],
      ]),
    );
    // Task 1's judge hold is history — only the current task narrates.
    expect(queryByTestId("chat-completion-1")).toBeNull();
    // Task 2 (not yet accepted) shows the waiting guidance.
    expect(queryByTestId("chat-completion-2")?.textContent).toContain(
      "You decide when this task is done.",
    );
  });
});
