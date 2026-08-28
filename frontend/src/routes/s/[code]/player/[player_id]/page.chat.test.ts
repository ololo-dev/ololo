import { describe, it, expect, beforeEach } from "vitest";
import { screen, fireEvent } from "@testing-library/svelte";
import { tick } from "svelte";
import { makeSnapshot, makeTaskSummary, makeProbe, renderPage } from "./page.test-helpers";

/**
 * The chat view: the session retold as a conversation. ololo hands out
 * tasks and runs its checks, the player answers with commits, done-notes
 * and captures, judges ask for evidence and reply with verdicts.
 */
describe("player page chat view", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  const snapshot = makeSnapshot({
    tasks: [
      makeTaskSummary({
        task_id: "task-1",
        ordinal: 1,
        title: "Build the widget",
        content: "Build a **weather widget**.",
        // For open-ended tasks this field carries the adapted check command,
        // not a brief — the chat must never show it as the task text.
        adapted_content: 'test -f .ololo/widget-done.md || echo "not-done"',
        result: {
          status: "completed",
          submitted_answer: null,
          correct_answer: null,
          score_delta: 10,
          evaluated_at: "2026-08-02T10:05:00Z",
        },
        scheduler_state: null,
        total_points: 42,
      }),
    ],
    probes: [
      // The same check polled three times — the chat collapses it into one
      // message carrying the latest state and the run count.
      makeProbe({
        id: "probe-1",
        task_id: "task-1",
        adapted_test_id: "done-flag",
        state: "resolved",
        outcome: "error",
        actual: "not-done: .ololo/widget-done.md is missing",
        test_command: "test -f .ololo/widget-done.md",
        rendered_command: "test -f .ololo/widget-done.md",
        dispatched_at: "2026-08-02T10:01:00Z",
        resolved_at: "2026-08-02T10:01:05Z",
        point_delta: 0,
      }),
      makeProbe({
        id: "probe-2",
        task_id: "task-1",
        adapted_test_id: "done-flag",
        state: "resolved",
        outcome: "error",
        actual: "not-done: .ololo/widget-done.md is missing",
        test_command: "test -f .ololo/widget-done.md",
        rendered_command: "test -f .ololo/widget-done.md",
        dispatched_at: "2026-08-02T10:02:00Z",
        resolved_at: "2026-08-02T10:02:05Z",
        point_delta: 0,
      }),
      makeProbe({
        id: "probe-3",
        task_id: "task-1",
        adapted_test_id: "done-flag",
        state: "resolved",
        outcome: "pass",
        actual: "done-note: present",
        label: "Definition of done",
        test_command: "test -f .ololo/widget-done.md",
        rendered_command: "test -f .ololo/widget-done.md",
        dispatched_at: "2026-08-02T10:04:00Z",
        resolved_at: "2026-08-02T10:04:05Z",
        point_delta: 25,
      }),
      // A judge's artifact request travels as a probe — the chat renders it
      // as the judge's message, deduped across retries.
      makeProbe({
        id: "probe-4",
        task_id: "task-1",
        adapted_test_id: "req-1",
        state: "resolved",
        outcome: "error",
        actual: "waiting-for-file: save the capture into .ololo/artifacts/req-1/",
        test_command:
          "# ARTIFACT REQUEST from ux-review: Please capture the widget as desktop.png. Do not run git commands.\n# Save the file(s) under .ololo/artifacts/req-1/",
        rendered_command: "# ARTIFACT REQUEST from ux-review: …",
        dispatched_at: "2026-08-02T10:03:00Z",
        resolved_at: "2026-08-02T10:03:10Z",
      }),
      makeProbe({
        id: "probe-5",
        task_id: "task-1",
        adapted_test_id: "req-1",
        state: "resolved",
        outcome: "pass",
        actual: "delivered",
        test_command:
          "# ARTIFACT REQUEST from ux-review: Please capture the widget as desktop.png\n# Save the file(s) under .ololo/artifacts/req-1/",
        rendered_command: "# ARTIFACT REQUEST from ux-review: …",
        dispatched_at: "2026-08-02T10:03:30Z",
        resolved_at: "2026-08-02T10:03:40Z",
      }),
      // Internal measurement probes carry no command — never shown.
      makeProbe({
        id: "probe-6",
        task_id: "task-1",
        adapted_test_id: "stats-probe",
        state: "resolved",
        outcome: "unavailable",
        actual: "",
        test_command: "",
        rendered_command: "",
        dispatched_at: "2026-08-02T10:04:30Z",
      }),
      // A request whose window closed with nothing delivered.
      makeProbe({
        id: "probe-r2",
        task_id: "task-1",
        adapted_test_id: "req-2",
        state: "resolved",
        outcome: "error",
        actual: "waiting-for-file: save the capture into .ololo/artifacts/req-2/",
        test_command:
          "# ARTIFACT REQUEST from test-quality: Please record a screencast as gameplay.webm\n# Save the file(s) under .ololo/artifacts/req-2/",
        rendered_command: "# ARTIFACT REQUEST from test-quality: …",
        dispatched_at: "2026-08-02T10:03:45Z",
        resolved_at: "2026-08-02T10:03:55Z",
      }),
      // A poll that keeps failing without scoring anything is the system
      // waiting on the player — never a message.
      makeProbe({
        id: "probe-8",
        task_id: "task-1",
        adapted_test_id: "waiting-poll",
        state: "resolved",
        outcome: "error",
        actual: "not-done: .ololo/other-done.md is missing",
        test_command: "test -f .ololo/other-done.md",
        rendered_command: "test -f .ololo/other-done.md",
        dispatched_at: "2026-08-02T10:04:50Z",
        resolved_at: "2026-08-02T10:04:55Z",
        point_delta: 0,
      }),
      // An LLM-rubric probe: the marker command means "an AI reviewer scored
      // the delivery files"; its answer is criterion → score JSON.
      makeProbe({
        id: "probe-7",
        task_id: "task-1",
        adapted_test_id: "rubric-1",
        state: "resolved",
        outcome: "pass",
        actual: '{"claims":9.5,"coverage":9.0,"readability":3.5}',
        test_command: "",
        rendered_command: "llm:rubric",
        dispatched_at: "2026-08-02T10:04:40Z",
        resolved_at: "2026-08-02T10:04:45Z",
      }),
    ],
    judge_results: [
      {
        task_id: "task-1",
        judge_slug: "correctness",
        judge_name: "Correctness",
        rating: 9,
        feedback: "Solid delivery.",
        point_delta: 27,
        created_at: "2026-08-02T10:06:00Z",
      },
    ],
    evaluations: [
      {
        task_id: "task-1",
        criteria: [],
        todo: {
          checked: 2,
          total: 2,
          items: [
            { text: "Fetch the forecast", done: true },
            { text: "Render the card", done: true },
          ],
        },
        artifacts: [
          {
            probe_id: "delivery-1",
            content_type: "image/png",
            label: ".ololo/artifacts/req-1/desktop.png",
            judge_slug: "ux-review",
            file_count: 1,
          },
        ],
      },
    ],
  });

  const history = {
    commits: [
      {
        sha: "aaaa111aaaa",
        author_name: "p",
        author_email: "p@example.com",
        author_time: "2026-08-02T10:03:50Z",
        message: "feat(task-1): weather widget with three city cards",
        files: [{ path: "index.html", status: "added", patch: "" }],
      },
      {
        sha: "bbbb222bbbb",
        author_name: "p",
        author_email: "p@example.com",
        author_time: "2026-08-02T10:04:10Z",
        message: "flag: widget-done.md",
        files: [
          {
            path: ".ololo/widget-done.md",
            status: "added",
            patch:
              "--- /dev/null\n+++ b/.ololo/widget-done.md\n@@ -0,0 +1 @@\n+Built a widget with three city cards and a forecast view.",
          },
        ],
      },
      // Machine traffic — never a chat message.
      {
        sha: "cccc333cccc",
        author_name: "p",
        author_email: "p@example.com",
        author_time: "2026-08-02T10:03:55Z",
        message: "artifact: sync",
        files: [{ path: ".ololo/artifacts/req-1/desktop.png", status: "added", patch: "" }],
      },
    ],
  };

  async function openChat() {
    renderPage({ live: true, snapshot, token: "t", playerName: "p", history });
    await fireEvent.click(screen.getByTestId("task-view-chat"));
    await tick();
  }

  it("Switches to the chat transcript", async () => {
    await openChat();
    expect(screen.getByTestId("task-chat")).not.toBeNull();
    expect(screen.queryByTestId("ptab-tasks")).toBeNull();
    expect(screen.getByTestId("task-view-chat").getAttribute("aria-pressed")).toBe("true");
  });

  it("ololo opens the conversation with the task brief, not the adapted check", async () => {
    await openChat();
    const brief = screen.getByTestId("chat-task-1");
    expect(brief.textContent).toContain("Build the widget");
    expect(brief.textContent).toContain("weather widget");
    expect(brief.textContent).not.toContain("not-done");
    // The task can be handed to an agent in one click.
    expect(screen.getByTestId("chat-copy-task-1")).not.toBeNull();
  });

  it("Bubble text renders as markdown — loose tables and line breaks included", async () => {
    const briefSnapshot = makeSnapshot({
      tasks: [
        makeTaskSummary({
          task_id: "task-1",
          ordinal: 1,
          title: "Build the game",
          // The house brief style: a pipe table without its separator row and
          // Gherkin lines that rely on single line breaks.
          content:
            "The shapes:\n\n  | shape | colour |\n  | I | cyan |\n  | O | yellow |\n\nScenario: A shape falls\n  When the game is running\n  Then the shape steps down",
          scheduler_state: { state: "active", activated_at: null, deadline_at: null },
        }),
      ],
    });
    renderPage({ live: true, snapshot: briefSnapshot, token: "t", playerName: "p" });
    await fireEvent.click(screen.getByTestId("task-view-chat"));
    await tick();
    const brief = screen.getByTestId("chat-task-1");
    expect(brief.querySelector("table")).not.toBeNull();
    expect(brief.textContent).toContain("cyan");
    expect(brief.querySelector("br")).not.toBeNull();
  });

  it("Collapses repeated probe runs into one check with the latest state", async () => {
    await openChat();
    const check = screen.getByTestId("chat-check-done-flag");
    expect(check.textContent).toContain("done-note: present");
    // The wire-carried test label heads the check.
    expect(check.textContent).toContain("Definition of done");
    expect(check.textContent).toContain("×3");
    expect(check.textContent).toContain("+25");
    // No forensics panel in the chat — Expected/Got/exit-code internals
    // live in the Details view.
    expect(check.querySelector("button")).toBeNull();
    expect(check.textContent).not.toContain("Expected:");
    expect(check.textContent).not.toContain("Exit code");
    // Internal measurement probes never render.
    expect(screen.queryByTestId("chat-check-stats-probe")).toBeNull();
    // Neither does a failing poll that scored nothing — the system is just
    // waiting on the player.
    expect(screen.queryByTestId("chat-check-waiting-poll")).toBeNull();
  });

  it("A scored failure quotes the literal expected answer inline", async () => {
    const quizSnapshot = makeSnapshot({
      tasks: [
        makeTaskSummary({
          task_id: "task-1",
          ordinal: 1,
          scheduler_state: { state: "active", activated_at: null, deadline_at: null },
        }),
      ],
      probes: [
        makeProbe({
          id: "probe-q",
          task_id: "task-1",
          adapted_test_id: "quiz-1",
          state: "resolved",
          outcome: "fail",
          actual: "7",
          expected: "42",
          test_command: "ask the answer",
          rendered_command:
            'curl -s -G "http://localhost:8088/" --data-urlencode "q=ab12cd: what is 6 multiplied by 7"',
          dispatched_at: "2026-08-02T10:01:00Z",
          resolved_at: "2026-08-02T10:01:05Z",
          point_delta: -5,
        }),
        // The CLI contract passes the question as a -q flag; qids there are
        // alphanumeric, not hex.
        makeProbe({
          id: "probe-q2",
          task_id: "task-1",
          adapted_test_id: "quiz-2",
          state: "resolved",
          outcome: "pass",
          actual: "4",
          test_command: "ask the answer",
          rendered_command: "R='sh answer.sh'\n$R -q \"jdla79to: what is 9 minus 5\"",
          dispatched_at: "2026-08-02T10:02:00Z",
          resolved_at: "2026-08-02T10:02:05Z",
          point_delta: 10,
        }),
      ],
    });
    renderPage({ live: true, snapshot: quizSnapshot, token: "t", playerName: "p" });
    await fireEvent.click(screen.getByTestId("task-view-chat"));
    await tick();
    const check = screen.getByTestId("chat-check-quiz-1");
    // The question the probe asked, without the hex qid plumbing.
    expect(check.textContent).toContain("what is 6 multiplied by 7");
    expect(check.textContent).not.toContain("ab12cd");
    expect(check.textContent).toContain("7");
    expect(check.textContent).toContain("expected");
    expect(check.textContent).toContain("42");
    expect(check.textContent).toContain("-5");
    // The CLI-contract form (-q flag, alphanumeric qid) is parsed too.
    const cliCheck = screen.getByTestId("chat-check-quiz-2");
    expect(cliCheck.textContent).toContain("what is 9 minus 5");
    expect(cliCheck.textContent).not.toContain("jdla79to");
  });

  it("An LLM-rubric probe joins the judges bubble as AI-review score chips", async () => {
    await openChat();
    // Not a standalone check any more — evaluation lives with the judges.
    expect(screen.queryByTestId("chat-check-rubric-1")).toBeNull();
    const judges = screen.getByTestId("chat-judges-1");
    const review = screen.getByTestId("chat-rubric-1");
    expect(judges.contains(review)).toBe(true);
    expect(review.textContent).toContain("AI review");
    expect(review.textContent).toContain("claims");
    expect(review.textContent).toContain("9.5");
    expect(review.textContent).toContain("readability");
    expect(review.textContent).not.toContain("llm:rubric");
  });

  it("Artifact requests group into one judge bubble, deduped, with statuses", async () => {
    await openChat();
    // One grouped bubble for the task, one row per distinct request.
    const group = screen.getByTestId("chat-requests-1");
    const requests = screen.getAllByTestId("chat-request-req-1");
    expect(requests.length).toBe(1);
    expect(group.contains(requests[0])).toBe(true);
    expect(requests[0].textContent).toContain("Please capture the widget as");
    expect(requests[0].textContent).toContain("delivered");
    // Delivery mechanics are the system's business — never shown.
    expect(requests[0].textContent).not.toContain("git");
    // The file name renders bold and the deliverable line spells it out.
    expect(requests[0].querySelector("strong")?.textContent).toBe("desktop.png");
    expect(requests[0].textContent).toContain("Create:");
    expect(requests[0].textContent).toContain(".ololo/artifacts/req-1/");
    // A request whose window closed empty says so instead of hanging open.
    const undelivered = screen.getByTestId("chat-request-req-2");
    expect(group.contains(undelivered)).toBe(true);
    expect(undelivered.textContent).toContain("not delivered");
  });

  it("The done-file lands as the player's text message; sync commits stay hidden", async () => {
    await openChat();
    const note = screen.getByTestId("chat-done-note-1");
    expect(note.textContent).toContain("Built a widget with three city cards");
    expect(screen.getByTestId("chat-commit-aaaa111").textContent).toContain(
      "weather widget with three city cards",
    );
    // The flag commit became the note; the artifact sync is machine traffic.
    expect(screen.queryByTestId("chat-commit-bbbb222")).toBeNull();
    expect(screen.queryByTestId("chat-commit-cccc333")).toBeNull();
  });

  it("The player's completion lands as a message with the plan", async () => {
    await openChat();
    const answer = screen.getByTestId("chat-answer-1");
    expect(answer.textContent).toContain("Task delivered");
    expect(answer.textContent).toContain("Render the card");
  });

  it("A task the server closed unfinished is ololo's line, not the player's", async () => {
    const closedSnapshot = makeSnapshot({
      tasks: [
        makeTaskSummary({
          task_id: "task-2",
          ordinal: 2,
          title: "Make it feel good",
          result: {
            status: "failed",
            submitted_answer: null,
            correct_answer: null,
            score_delta: 0,
            evaluated_at: "2026-08-02T11:00:00Z",
          },
          scheduler_state: null,
        }),
      ],
      probes: [
        makeProbe({
          id: "probe-9",
          task_id: "task-2",
          adapted_test_id: "done-flag-2",
          dispatched_at: "2026-08-02T10:30:00Z",
        }),
      ],
    });
    renderPage({ live: true, snapshot: closedSnapshot, token: "t", playerName: "p" });
    await fireEvent.click(screen.getByTestId("task-view-chat"));
    await tick();
    const closed = screen.getByTestId("chat-closed-2");
    expect(closed.textContent).toContain("ololo");
    expect(closed.textContent).toContain("Task #2 closed unfinished");
    expect(screen.queryByTestId("chat-answer-2")).toBeNull();
  });

  it("Judges reply as one bubble of verdict chips", async () => {
    await openChat();
    const judges = screen.getByTestId("chat-judges-1");
    const chip = screen.getByTestId("chat-judge-chip-correctness-task-1");
    expect(judges.contains(chip)).toBe(true);
    expect(chip.textContent).toContain("Correctness");
    expect(chip.textContent).toContain("+27");
    // The feedback lives in the hover card, not inline.
    expect(judges.textContent).not.toContain("Solid delivery.");
  });

  it("A failed attempt on a still-scheduled task reads as a retry, not a closure", async () => {
    const retrySnapshot = makeSnapshot({
      tasks: [
        makeTaskSummary({
          task_id: "task-3",
          ordinal: 3,
          title: "Switch cities",
          result: {
            status: "failed",
            submitted_answer: null,
            correct_answer: null,
            score_delta: 0,
            evaluated_at: "2026-08-02T10:20:00Z",
          },
          // The scheduler still holds the task — a retry is coming.
          scheduler_state: {
            state: "idle",
            activated_at: null,
            deadline_at: "2026-08-02T10:25:00Z",
          },
        }),
      ],
      probes: [
        makeProbe({
          id: "probe-r",
          task_id: "task-3",
          adapted_test_id: "done-flag-3",
          dispatched_at: "2026-08-02T10:15:00Z",
        }),
      ],
    });
    renderPage({ live: true, snapshot: retrySnapshot, token: "t", playerName: "p" });
    await fireEvent.click(screen.getByTestId("task-view-chat"));
    await tick();
    expect(screen.queryByTestId("chat-closed-3")).toBeNull();
    expect(screen.getByTestId("chat-retry-3").textContent).toContain("retry scheduled");
  });

  it("Verdicts close the task — a re-request never lands below the verdict it preceded", async () => {
    const reorderSnapshot = makeSnapshot({
      tasks: [
        makeTaskSummary({
          task_id: "task-1",
          ordinal: 1,
          scheduler_state: { state: "active", activated_at: null, deadline_at: null },
        }),
      ],
      probes: [
        // The judge re-requested evidence AFTER scoring; the chat must still
        // read ask → evaluate.
        makeProbe({
          id: "probe-late-req",
          task_id: "task-1",
          adapted_test_id: "req-late",
          state: "resolved",
          outcome: "error",
          actual: "waiting-for-file",
          test_command:
            "# ARTIFACT REQUEST from ux-review: One more screencast please\n# Save under .ololo/artifacts/req-late/",
          rendered_command: "# ARTIFACT REQUEST from ux-review: …",
          dispatched_at: "2026-08-02T10:07:00Z",
        }),
      ],
      judge_results: [
        {
          task_id: "task-1",
          judge_slug: "ux-review",
          judge_name: "UX Review",
          rating: 8,
          feedback: "Looks good.",
          point_delta: 19,
          created_at: "2026-08-02T10:02:00Z",
        },
      ],
    });
    renderPage({ live: true, snapshot: reorderSnapshot, token: "t", playerName: "p" });
    await fireEvent.click(screen.getByTestId("task-view-chat"));
    await tick();
    const request = screen.getByTestId("chat-request-req-late");
    const verdict = screen.getByTestId("chat-judges-1");
    const order = request.compareDocumentPosition(verdict);
    expect(order & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("A finished session closes with a summary card", async () => {
    const finishedSnapshot = makeSnapshot({
      score: 350,
      rank: 2,
      total_tasks: 3,
      session_status: "finished",
      tasks: [
        makeTaskSummary({
          task_id: "task-1",
          ordinal: 1,
          result: {
            status: "completed",
            submitted_answer: null,
            correct_answer: null,
            score_delta: 10,
            evaluated_at: "2026-08-02T10:05:00Z",
          },
          scheduler_state: null,
          bonus_points: 10,
        }),
        makeTaskSummary({
          task_id: "task-2",
          ordinal: 2,
          result: {
            status: "completed",
            submitted_answer: null,
            correct_answer: null,
            score_delta: 10,
            evaluated_at: "2026-08-02T10:15:00Z",
          },
          scheduler_state: null,
          bonus_points: 10,
        }),
      ],
      probes: [
        makeProbe({
          id: "probe-a",
          task_id: "task-1",
          adapted_test_id: "t-a",
          state: "resolved",
          outcome: "pass",
          dispatched_at: "2026-08-02T10:01:00Z",
          point_delta: 25,
        }),
        makeProbe({
          id: "probe-b",
          task_id: "task-2",
          adapted_test_id: "t-b",
          state: "resolved",
          outcome: "pass",
          dispatched_at: "2026-08-02T10:11:00Z",
          point_delta: 15,
        }),
      ],
      judge_results: [
        {
          task_id: "task-1",
          judge_slug: "correctness",
          judge_name: "Correctness",
          rating: 9,
          feedback: "",
          point_delta: 30,
          created_at: "2026-08-02T10:06:00Z",
        },
        {
          task_id: "task-2",
          judge_slug: "correctness",
          judge_name: "Correctness",
          rating: 8,
          feedback: "",
          point_delta: 20,
          created_at: "2026-08-02T10:16:00Z",
        },
        {
          task_id: "task-1",
          judge_slug: "ux-review",
          judge_name: "UX Review",
          rating: 7,
          feedback: "",
          point_delta: 18,
          created_at: "2026-08-02T10:06:30Z",
        },
      ],
      evaluations: [
        {
          task_id: "task-1",
          criteria: [
            {
              key: "polish",
              title: "Polish",
              weight: 1,
              scores: [
                { judge_slug: "ux-review", score: 8, rationale: "" },
                { judge_slug: "correctness", score: 6, rationale: "" },
              ],
            },
          ],
        },
      ],
    });
    renderPage({ live: false, snapshot: finishedSnapshot, token: null, playerName: "p" });
    await fireEvent.click(screen.getByTestId("task-view-chat"));
    await tick();
    const card = screen.getByTestId("chat-summary");
    expect(card.textContent).toContain("Session summary");
    expect(card.textContent).toContain("350");
    expect(card.textContent).toContain("#2");
    expect(card.textContent).toContain("2/3");
    expect(card.textContent).toContain("2");
    // Checks +40, bonus +20, judges +68 total.
    expect(card.textContent).toContain("Check pts");
    expect(card.textContent).toContain("+40");
    expect(card.textContent).toContain("+20");
    expect(card.textContent).toContain("+68");
    // Judges section counts its evaluations.
    expect(card.textContent).toMatch(/3\s+evaluations/);
    // Per-judge totals across tasks, with the verdict count.
    const correctness = screen.getByTestId("chat-summary-judge-correctness");
    expect(correctness.textContent).toContain("+50");
    expect(correctness.textContent).toContain("×2");
    const ux = screen.getByTestId("chat-summary-judge-ux-review");
    expect(ux.textContent).toContain("+18");
    expect(ux.textContent).toContain("×1");
    // Criteria averages.
    expect(card.textContent).toContain("Polish");
    expect(card.textContent).toContain("7.0");
  });

  it("Remembers the choice across reloads", async () => {
    const first = renderPage({ live: true, snapshot, token: "t", playerName: "p" });
    await fireEvent.click(screen.getByTestId("task-view-chat"));
    await tick();
    first.unmount();

    renderPage({ live: true, snapshot, token: "t", playerName: "p" });
    expect(screen.getByTestId("task-chat")).not.toBeNull();
    expect(screen.getByTestId("task-view-chat").getAttribute("aria-pressed")).toBe("true");
  });
});
