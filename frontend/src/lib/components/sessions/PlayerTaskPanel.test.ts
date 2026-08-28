import { describe, it, expect, vi } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { tick } from "svelte";
import PlayerTaskPanel from "./PlayerTaskPanel.svelte";
import type {
  PlayerTaskSummaryEntry,
  PlayerProbeEntry,
  PlayerHistoryCommit,
  TaskStatsEntry,
} from "$lib/types/arena";

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
    point_delta: 10,
    updated_at: null,
    ...over,
  };
}

function mkCommit(
  files: { path: string; status: "added" | "modified" | "deleted"; patch: string }[] = [],
): PlayerHistoryCommit {
  return {
    sha: "c-" + Math.random().toString(36).slice(2, 6),
    author_name: "bot",
    author_email: "b@x",
    author_time: "2026-01-01T00:00:00Z",
    message: "feat(t1): x",
    files,
  };
}

describe("PlayerTaskPanel", () => {
  it("renders header with ordinal, title, section chips, probe count", () => {
    const task = mkTask({ ordinal: 2, title: "Parser", tags: ["rust", "cli"] });
    const commits = [mkCommit([{ path: "a.txt", status: "added", patch: "+a" }])];
    const { container, getByText, getByTestId } = render(PlayerTaskPanel, {
      props: {
        task,
        probes: [mkProbe()],
        statusBadge: "completed",
        passedTypes: 1,
        totalTypes: 1,
        changes: { commits, mode: "per-commit" },
        defaultExpanded: true,
      },
    });
    expect(container.textContent).toContain("#2");
    expect(getByText("Parser")).toBeTruthy();
    // Tags are no longer shown in the header — section chips are.
    expect(container.textContent).not.toContain("rust");
    const sections = getByTestId("task-sections-2");
    expect(sections.textContent).toContain("probes (1)");
    expect(sections.textContent).toContain("changes");
    expect(sections.textContent).not.toContain("judges");
    expect(sections.textContent).not.toContain("stats");
    expect(getByText("completed")).toBeTruthy();
  });

  it("renders bonus points chip when present", () => {
    const task = mkTask({ ordinal: 4, bonus_points: 20, total_points: 65 });
    const { getByTestId } = render(PlayerTaskPanel, {
      props: {
        task,
        probes: [mkProbe()],
        statusBadge: "completed",
        passedTypes: 1,
        totalTypes: 1,
        changes: { commits: [], mode: "per-commit" },
        defaultExpanded: false,
      },
    });
    expect(getByTestId("task-bonus-points-4").textContent).toContain("bonus +20");
    expect(getByTestId("task-total-points-4").textContent).toContain("+65 pts");
  });

  it("default tab is Probes", () => {
    const task = mkTask();
    const commits = [mkCommit([{ path: "a.txt", status: "added", patch: "+a" }])];
    const { container } = render(PlayerTaskPanel, {
      props: {
        task,
        probes: [mkProbe()],
        statusBadge: "pending",
        passedTypes: 0,
        totalTypes: 1,
        changes: { commits, mode: "per-commit" },
        defaultExpanded: true,
      },
    });
    const probeTab = container.querySelector('[data-testid="tab-probes"]');
    const changesTab = container.querySelector('[data-testid="tab-changes"]');
    expect(probeTab?.getAttribute("aria-selected")).toBe("true");
    expect(changesTab?.getAttribute("aria-selected")).toBe("false");
  });

  it("data-dependent tabs hidden without data", () => {
    const task = mkTask();
    const { container } = render(PlayerTaskPanel, {
      props: {
        task,
        probes: [mkProbe()],
        statusBadge: "pending",
        passedTypes: 0,
        totalTypes: 1,
        changes: { commits: [], mode: "per-commit" },
        defaultExpanded: true,
      },
    });
    expect(container.querySelector('[data-testid="tab-probes"]')).toBeTruthy();
    expect(container.querySelector('[data-testid="tab-changes"]')).toBeNull();
    expect(container.querySelector('[data-testid="tab-judges"]')).toBeNull();
    expect(container.querySelector('[data-testid="tab-stats"]')).toBeNull();
  });

  it("judges tab appears when results arrive", () => {
    const task = mkTask();
    const judgeResults = [
      {
        task_id: "t1",
        judge_slug: "anti-cheater",
        judge_name: "Anti Cheater",
        rating: 0,
        feedback: "clean",
        point_delta: 0,
        created_at: "2026-01-01T00:00:00Z",
      },
    ];
    const { container } = render(PlayerTaskPanel, {
      props: {
        task,
        probes: [mkProbe()],
        statusBadge: "pending",
        passedTypes: 0,
        totalTypes: 1,
        changes: { commits: [], mode: "per-commit" },
        defaultExpanded: true,
        judgeResults,
      },
    });
    const judgesTab = container.querySelector('[data-testid="tab-judges"]');
    expect(judgesTab).toBeTruthy();
    expect(judgesTab?.textContent).toContain("(1/1)");
  });

  it("judges tab appears from statuses alone and shows lifecycle states", async () => {
    const task = mkTask();
    const judgeStatuses = [
      {
        task_id: "t1",
        judge_slug: "code-cleanliness",
        judge_name: "Code Cleanliness",
        status: "running",
        error: null,
        updated_at: "2026-01-01T00:00:00Z",
      },
      {
        task_id: "t1",
        judge_slug: "anti-cheater",
        judge_name: "Anti Cheater",
        status: "failed",
        error: "ai_timeout",
        updated_at: "2026-01-01T00:00:05Z",
      },
      {
        task_id: "t1",
        judge_slug: "test-coverage",
        judge_name: "Test Coverage",
        status: "pending",
        error: null,
        updated_at: null,
      },
    ];
    const { container } = render(PlayerTaskPanel, {
      props: {
        task,
        probes: [mkProbe()],
        statusBadge: "pending",
        passedTypes: 0,
        totalTypes: 1,
        changes: { commits: [], mode: "per-commit" },
        defaultExpanded: true,
        judgeStatuses,
      },
    });
    const judgesTab = container.querySelector('[data-testid="tab-judges"]');
    expect(judgesTab).toBeTruthy();
    expect(judgesTab?.textContent).toContain("(0/3)");
    await fireEvent.click(judgesTab!);
    const panel = container.querySelector('[data-testid="task-judges-1"]');
    expect(panel?.textContent).toContain("Evaluating…");
    expect(panel?.textContent).toContain("Failed");
    expect(panel?.textContent).toContain("ai_timeout");
    expect(panel?.textContent).toContain("Pending");
    expect(panel?.textContent).toContain("Runs automatically once this task completes.");
  });

  it("scored status row renders the verdict with rating and feedback", async () => {
    const task = mkTask();
    const judgeStatuses = [
      {
        task_id: "t1",
        judge_slug: "code-cleanliness",
        judge_name: "Code Cleanliness",
        status: "scored",
        error: null,
        updated_at: "2026-01-01T00:00:10Z",
      },
    ];
    const judgeResults = [
      {
        task_id: "t1",
        judge_slug: "code-cleanliness",
        judge_name: "Code Cleanliness",
        rating: 4,
        feedback: "tidy work",
        point_delta: 8,
        created_at: "2026-01-01T00:00:10Z",
      },
    ];
    const { container } = render(PlayerTaskPanel, {
      props: {
        task,
        probes: [mkProbe()],
        statusBadge: "pending",
        passedTypes: 0,
        totalTypes: 1,
        changes: { commits: [], mode: "per-commit" },
        defaultExpanded: true,
        judgeResults,
        judgeStatuses,
      },
    });
    const judgesTab = container.querySelector('[data-testid="tab-judges"]');
    expect(judgesTab?.textContent).toContain("(1/1)");
    await fireEvent.click(judgesTab!);
    const panel = container.querySelector('[data-testid="task-judges-1"]');
    expect(panel?.textContent).toContain("Rating 4");
    expect(panel?.textContent).toContain("+8 pts");
    expect(panel?.textContent).toContain("tidy work");
  });

  it("tab toggle switches content to Changes", async () => {
    const task = mkTask();
    const commits = [mkCommit([{ path: "a.txt", status: "added", patch: "+a" }])];
    const { container, getByTestId } = render(PlayerTaskPanel, {
      props: {
        task,
        probes: [mkProbe()],
        statusBadge: "pending",
        passedTypes: 0,
        totalTypes: 1,
        changes: { commits, mode: "range" },
        defaultExpanded: true,
      },
    });
    const changesTab = getByTestId("tab-changes");
    await fireEvent.click(changesTab);
    expect(changesTab.getAttribute("aria-selected")).toBe("true");
    expect(container.querySelector('[data-testid="task-changes-1"]')).toBeTruthy();
  });

  it("probes tab shows probe count", () => {
    const task = mkTask();
    const { container } = render(PlayerTaskPanel, {
      props: {
        task,
        probes: [
          mkProbe({ id: "p1", attempt: 1 }),
          mkProbe({ id: "p2", attempt: 2 }),
          mkProbe({ id: "p3", attempt: 3 }),
        ],
        statusBadge: "pending",
        passedTypes: 0,
        totalTypes: 1,
        changes: { commits: [], mode: "per-commit" },
        defaultExpanded: true,
      },
    });
    const probesTab = container.querySelector('[data-testid="tab-probes"]');
    expect(probesTab?.textContent).toContain("Probes");
    expect(probesTab?.textContent).toContain("(3)");
  });

  it("header pulses on new probes and point changes, not on initial render", async () => {
    const task = mkTask({ ordinal: 5, total_points: 10 });
    const baseProps = {
      task,
      probes: [mkProbe({ id: "p1", attempt: 1 })],
      statusBadge: "pending",
      passedTypes: 0,
      totalTypes: 1,
      changes: { commits: [], mode: "per-commit" as const },
      defaultExpanded: false,
    };
    const { container, rerender } = render(PlayerTaskPanel, { props: baseProps });

    // No pulse on first render.
    expect(container.querySelector(".chip-pulse")).toBeNull();
    expect(container.querySelector(".points-pulse")).toBeNull();

    // New probe arrives → probes chip pulses.
    await rerender({
      ...baseProps,
      probes: [mkProbe({ id: "p1", attempt: 1 }), mkProbe({ id: "p2", attempt: 2 })],
    });
    const sections = container.querySelector('[data-testid="task-sections-5"]');
    expect(sections?.querySelector(".chip-pulse")?.textContent).toContain("probes (2)");

    // Total points change → points pulse.
    await rerender({
      ...baseProps,
      probes: [mkProbe({ id: "p1", attempt: 1 }), mkProbe({ id: "p2", attempt: 2 })],
      task: { ...task, total_points: 25 },
    });
    const points = container.querySelector('[data-testid="task-total-points-5"]');
    expect(points?.classList.contains("points-pulse")).toBe(true);
    expect(points?.textContent).toContain("+25 pts");
  });

  it("probes ordered by dispatched_at desc", () => {
    const task = mkTask();
    const probes = [
      mkProbe({ id: "p1", dispatched_at: "2026-01-01T00:00:00Z", attempt: 1 }),
      mkProbe({ id: "p2", dispatched_at: "2026-01-02T00:00:00Z", attempt: 2 }),
      mkProbe({ id: "p3", dispatched_at: "2026-01-01T12:00:00Z", attempt: 3 }),
    ];
    const { container } = render(PlayerTaskPanel, {
      props: {
        task,
        probes,
        statusBadge: "pending",
        passedTypes: 0,
        totalTypes: 3,
        changes: { commits: [], mode: "per-commit" },
        defaultExpanded: true,
      },
    });
    const rows = Array.from(container.querySelectorAll('[data-testid^="probe-row-"]'));
    expect(rows.length).toBe(3);
    expect(rows[0].getAttribute("data-testid")).toBe("probe-row-p2");
    expect(rows[1].getAttribute("data-testid")).toBe("probe-row-p3");
    expect(rows[2].getAttribute("data-testid")).toBe("probe-row-p1");
  });

  it("probe row shows probe type from test_ordinal", () => {
    const task = mkTask();
    const { getByTestId } = render(PlayerTaskPanel, {
      props: {
        task,
        probes: [mkProbe({ id: "p1", test_ordinal: 2, attempt: 3 })],
        statusBadge: "pending",
        passedTypes: 0,
        totalTypes: 1,
        changes: { commits: [], mode: "per-commit" },
        defaultExpanded: true,
      },
    });
    const row = getByTestId("probe-row-p1");
    // test_ordinal is 0-based → displayed as "Probe type 3".
    expect(row.textContent).toContain("Probe type 3");
    expect(row.textContent).toContain("attempt 3");
    expect(row.textContent).not.toContain("Probe #");
  });

  it("probe row falls back to Probe #attempt without test_ordinal", () => {
    const task = mkTask();
    const { getByTestId } = render(PlayerTaskPanel, {
      props: {
        task,
        probes: [mkProbe({ id: "p1", test_ordinal: null, attempt: 4 })],
        statusBadge: "pending",
        passedTypes: 0,
        totalTypes: 1,
        changes: { commits: [], mode: "per-commit" },
        defaultExpanded: true,
      },
    });
    expect(getByTestId("probe-row-p1").textContent).toContain("Probe #4");
  });

  it("secret-expected probe shows Response note, not a bare Actual row", async () => {
    const task = mkTask();
    // Expected value hidden: probe.expected_answer null, result.expected null,
    // but the player's response (result.actual) is present.
    const probe = mkProbe({
      id: "p1",
      expected_answer: null,
      expected: null,
      actual: "2017",
      result: { status: "failed", expected: null, actual: "2017" },
      outcome: "error",
    });
    const { container, getByTestId } = render(PlayerTaskPanel, {
      props: {
        task,
        probes: [probe],
        statusBadge: "pending",
        passedTypes: 0,
        totalTypes: 1,
        changes: { commits: [], mode: "per-commit" },
        defaultExpanded: true,
      },
    });
    // Ensure the row is expanded.
    const row = getByTestId("probe-row-p1");
    if (!container.textContent?.includes("expected value is hidden")) {
      await fireEvent.click(row.querySelector("button")!);
    }
    expect(container.textContent).toContain("Response");
    expect(container.textContent).toContain("2017");
    expect(container.textContent).toContain("expected value is hidden for this question");
    // The misleading "Expected Answers" heading must not appear for this probe.
    expect(container.textContent).not.toContain("Expected Answers");
  });

  it("probe expand shows full info", async () => {
    const task = mkTask();
    const probe = mkProbe({
      id: "p1",
      rendered_command: "echo VERY-DETAILED-CMD",
      outcome: "pass",
    });
    const { container, getByTestId } = render(PlayerTaskPanel, {
      props: {
        task,
        probes: [probe],
        statusBadge: "pending",
        passedTypes: 0,
        totalTypes: 1,
        changes: { commits: [], mode: "per-commit" },
        defaultExpanded: true,
      },
    });
    // Latest probe auto-expanded — command visible immediately
    expect(container.textContent).toContain("VERY-DETAILED-CMD");
    const probeRow = getByTestId("probe-row-p1");
    const toggle = probeRow.querySelector("button");
    await fireEvent.click(toggle!);
    expect(container.textContent).not.toContain("VERY-DETAILED-CMD");
  });

  it("empty changes → no Changes tab", () => {
    const task = mkTask();
    const { container } = render(PlayerTaskPanel, {
      props: {
        task,
        probes: [mkProbe()],
        statusBadge: "pending",
        passedTypes: 0,
        totalTypes: 1,
        changes: { commits: [], mode: "per-commit" },
        defaultExpanded: true,
      },
    });
    expect(container.querySelector('[data-testid="tab-changes"]')).toBeNull();
  });

  it("ARIA roles present on tabs; only data-backed tabs render", () => {
    const task = mkTask();
    const { container } = render(PlayerTaskPanel, {
      props: {
        task,
        probes: [mkProbe()],
        statusBadge: "pending",
        passedTypes: 0,
        totalTypes: 1,
        changes: { commits: [], mode: "per-commit" },
        defaultExpanded: true,
      },
    });
    expect(container.querySelector('[role="tablist"]')).toBeTruthy();
    // Only Probes is data-backed in this render.
    expect(container.querySelectorAll('[role="tab"]').length).toBe(1);
    expect(container.querySelector('[role="tabpanel"]')).toBeTruthy();
  });

  it("collapsed by default when defaultExpanded=false", () => {
    const task = mkTask();
    const { container } = render(PlayerTaskPanel, {
      props: {
        task,
        probes: [mkProbe()],
        statusBadge: "pending",
        passedTypes: 0,
        totalTypes: 1,
        changes: { commits: [], mode: "per-commit" },
        defaultExpanded: false,
      },
    });
    const panel = container.querySelector('[data-testid^="task-panel-"]');
    expect(panel).toBeTruthy();
    expect(panel?.querySelector('[role="tablist"]')).toBeNull();
  });

  it("expanded by default when defaultExpanded=true", () => {
    const task = mkTask();
    const { container } = render(PlayerTaskPanel, {
      props: {
        task,
        probes: [mkProbe()],
        statusBadge: "pending",
        passedTypes: 0,
        totalTypes: 1,
        changes: { commits: [], mode: "per-commit" },
        defaultExpanded: true,
      },
    });
    expect(container.querySelector('[role="tablist"]')).toBeTruthy();
  });

  it("stats tab hidden without stats", () => {
    const task = mkTask();
    const { container } = render(PlayerTaskPanel, {
      props: {
        task,
        probes: [mkProbe()],
        statusBadge: "pending",
        passedTypes: 0,
        totalTypes: 1,
        changes: { commits: [], mode: "per-commit" },
        defaultExpanded: true,
      },
    });
    expect(container.querySelector('[data-testid="tab-stats"]')).toBeNull();
  });

  it("stats tab renders totals, tools, and skills", async () => {
    const task = mkTask();
    const stats: TaskStatsEntry = {
      task_id: "t1",
      task_ordinal: 1,
      window_started_at: "2026-01-01T00:00:00Z",
      window_ended_at: "2026-01-01T00:02:30Z",
      input_tokens: 12_500,
      output_tokens: 3_400,
      cache_read_tokens: 1_000_000,
      cache_write_tokens: 50_000,
      reasoning_tokens: 0,
      cost: 0.1234,
      user_messages: 2,
      assistant_messages: 14,
      tool_calls: 9,
      agents: [
        {
          agent: "claude",
          agent_session_id: "sess-abc",
          model: "claude-fable-5",
          input_tokens: 12_500,
          output_tokens: 3_400,
          cache_read_tokens: 1_000_000,
          cache_write_tokens: 50_000,
          reasoning_tokens: 0,
          cost: 0.1234,
          user_messages: 2,
          assistant_messages: 14,
          tool_calls: 9,
          tools: { Bash: 5, Edit: 3, Read: 1 },
          skills: { "drill-tdd": 1 },
        },
      ],
      created_at: "2026-01-01T00:03:00Z",
      updated_at: "2026-01-01T00:03:00Z",
    };
    const { container, getByTestId } = render(PlayerTaskPanel, {
      props: {
        task,
        probes: [mkProbe()],
        statusBadge: "pending",
        passedTypes: 0,
        totalTypes: 1,
        changes: { commits: [], mode: "per-commit" },
        defaultExpanded: true,
        stats,
      },
    });
    const statsTab = getByTestId("tab-stats");
    expect(statsTab.textContent).toContain("(1)");
    await fireEvent.click(statsTab);
    const panel = getByTestId("task-stats-1");
    // Summary tiles
    expect(panel.textContent).toContain("1.1M / 3.4k"); // total fed in (cache included) /out
    expect(panel.textContent).toContain("1.0M / 50.0k"); // cache r/w
    expect(panel.textContent).toContain("2 / 14"); // messages
    expect(panel.textContent).toContain("$0.1234");
    expect(panel.textContent).toContain("2m 30s"); // implementation time
    // Per-agent breakdown
    expect(panel.textContent).toContain("claude");
    expect(panel.textContent).toContain("claude-fable-5");
    expect(panel.textContent).toContain("sess-abc");
    expect(panel.textContent).toContain("Bash");
    expect(panel.textContent).toContain("drill-tdd");
  });

  it("click header toggles expand/collapse", async () => {
    const task = mkTask();
    const { container } = render(PlayerTaskPanel, {
      props: {
        task,
        probes: [mkProbe()],
        statusBadge: "pending",
        passedTypes: 0,
        totalTypes: 1,
        changes: { commits: [], mode: "per-commit" },
        defaultExpanded: false,
      },
    });
    expect(container.querySelector('[role="tablist"]')).toBeNull();
    const header = container.querySelector("button");
    await fireEvent.click(header!);
    expect(container.querySelector('[role="tablist"]')).toBeTruthy();
  });

  it("clicking a section badge expands the panel and opens that tab", async () => {
    const task = mkTask();
    const judgeResults = [
      {
        task_id: "t1",
        judge_slug: "anti-cheater",
        judge_name: "Anti Cheater",
        rating: 0,
        feedback: "clean",
        point_delta: 0,
        created_at: "2026-01-01T00:00:00Z",
      },
    ];
    const { container } = render(PlayerTaskPanel, {
      props: {
        task,
        probes: [mkProbe()],
        statusBadge: "pending",
        passedTypes: 0,
        totalTypes: 1,
        changes: { commits: [], mode: "per-commit" },
        defaultExpanded: false,
        judgeResults,
      },
    });

    // Collapsed: no tab strip at all.
    expect(container.querySelector('[role="tablist"]')).toBeNull();

    await fireEvent.click(container.querySelector('[data-testid="task-section-judges-1"]')!);

    // One click both reveals the body and lands on the named tab, rather
    // than opening on the default Probes tab.
    expect(container.querySelector('[role="tablist"]')).toBeTruthy();
    expect(
      container.querySelector('[data-testid="tab-judges"]')?.getAttribute("aria-selected"),
    ).toBe("true");
    expect(container.querySelector('[data-testid="task-judges-1"]')).toBeTruthy();
  });

  it("a section badge keeps an open panel open instead of toggling it shut", async () => {
    const task = mkTask();
    const { container } = render(PlayerTaskPanel, {
      props: {
        task,
        probes: [mkProbe()],
        statusBadge: "pending",
        passedTypes: 0,
        totalTypes: 1,
        changes: { commits: [], mode: "per-commit" },
        defaultExpanded: true,
      },
    });
    expect(container.querySelector('[role="tablist"]')).toBeTruthy();

    // The badge sits inside the header next to a toggle; if its click bubbled
    // into that toggle the panel would collapse under the click.
    await fireEvent.click(container.querySelector('[data-testid="task-section-probes-1"]')!);
    expect(container.querySelector('[role="tablist"]')).toBeTruthy();
    expect(
      container.querySelector('[data-testid="tab-probes"]')?.getAttribute("aria-selected"),
    ).toBe("true");
  });

  it("omits the attempt number on a first attempt", () => {
    const task = mkTask();
    const { container } = render(PlayerTaskPanel, {
      props: {
        task,
        probes: [mkProbe({ id: "p1", attempt: 1, test_ordinal: null })],
        statusBadge: "pending",
        passedTypes: 0,
        totalTypes: 1,
        changes: { commits: [], mode: "per-commit" },
        defaultExpanded: true,
      },
    });
    const row = container.querySelector('[data-testid="probe-row-p1"]');
    expect(row?.textContent).toContain("Probe");
    expect(row?.textContent).not.toContain("#1");
    expect(row?.textContent).not.toContain("attempt");
  });
  it("Hands the probe and its task to an agent in one copy", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText } });

    const { container } = render(PlayerTaskPanel, {
      props: {
        task: mkTask({ ordinal: 3, title: "Parser", content: "Parse the input." }),
        probes: [mkProbe({ id: "p9", rendered_command: "sh answer.sh 42" })],
        // Not "completed": that collapses the whole panel on purpose.
        statusBadge: "pending",
        passedTypes: 1,
        totalTypes: 1,
        changes: { commits: [], mode: "per-commit" },
        defaultExpanded: true,
      },
    });
    // Open the probe body, whether or not the auto-expand effect got there.
    await tick();
    if (!container.querySelector('[data-testid="copy-briefing-p9"]')) {
      await fireEvent.click(container.querySelector('[data-testid="probe-row-p9"] button')!);
    }
    await fireEvent.click(container.querySelector('[data-testid="copy-briefing-p9"]')!);

    const text = writeText.mock.calls[0][0] as string;
    // The same briefing the slides view produces, from whichever view the
    // player happens to be in.
    expect(text).toContain("## Task 3: Parser");
    expect(text).toContain("Parse the input.");
    expect(text).toContain("### Probe: Passed");
    expect(text).toContain("sh answer.sh 42");
    vi.unstubAllGlobals();
  });
});
