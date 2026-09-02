// @ts-nocheck — Storybook v10 types don't yet fully support Svelte 5 runes-mode components.
import type { Meta, StoryObj } from "@storybook/sveltekit";
import PlayerTaskChat from "$lib/components/sessions/PlayerTaskChat.svelte";

const tasks = [
  {
    task_id: "t1",
    ordinal: 1,
    title: "Build the weather widget",
    content:
      "Build a weather widget: a web page that shows the current weather for a requested city.\n\nWhen you are done, write `.ololo/weather-widget-done.md` with a short description of what you built.",
    adapted_content: "",
    tags: [],
    result: {
      status: "completed",
      submitted_answer: null,
      correct_answer: null,
      score_delta: 10,
      evaluated_at: "2026-08-10T10:04:00Z",
    },
    scheduler_state: null,
    total_points: 167,
  },
  {
    task_id: "t2",
    ordinal: 2,
    title: "Switch cities and open the forecast",
    content:
      "Grow the widget: a visitor moves between cities without touching the address bar, and every city opens into a full three-day forecast.",
    adapted_content: "",
    tags: [],
    result: null,
    scheduler_state: { state: "active", activated_at: "2026-08-10T10:05:00Z", deadline_at: null },
    total_points: 24,
  },
];

const probesByTask = new Map([
  [
    "t1",
    [
      {
        id: "p1",
        task_id: "t1",
        task_title: "Build the weather widget",
        task_ordinal: 1,
        adapted_test_id: "done-flag",
        test_command: "cat .ololo/weather-widget-done.md",
        attempt: 1,
        rendered_command: "cat .ololo/weather-widget-done.md",
        fixture_values: null,
        expected_answer: null,
        state: "resolved",
        outcome: "pass",
        actual: "Built a widget with three city cards.",
        expected: null,
        exit_code: 0,
        duration_ms: 412,
        dispatched_at: "2026-08-10T10:02:00Z",
        deadline_at: null,
        resolved_at: "2026-08-10T10:02:01Z",
        result: { status: "passed", expected: null, actual: null },
        point_delta: 25,
        updated_at: null,
      },
    ],
  ],
  [
    "t2",
    [
      {
        id: "p2",
        task_id: "t2",
        task_title: "Switch cities and open the forecast",
        task_ordinal: 2,
        adapted_test_id: "done-flag",
        test_command: "cat .ololo/forecast-done.md",
        attempt: 1,
        rendered_command: "cat .ololo/forecast-done.md",
        fixture_values: null,
        expected_answer: null,
        state: "dispatched",
        outcome: null,
        actual: null,
        expected: null,
        exit_code: null,
        duration_ms: null,
        dispatched_at: "2026-08-10T10:06:00Z",
        deadline_at: "2026-08-10T10:16:00Z",
        resolved_at: null,
        result: null,
        point_delta: 0,
        updated_at: null,
      },
    ],
  ],
]);

const judgeResultsByTask = new Map([
  [
    "t1",
    [
      {
        task_id: "t1",
        judge_slug: "correctness",
        judge_name: "Correctness",
        rating: 9,
        feedback:
          "The widget renders every requested city and the forecast matches the dataset. **Well structured** delivery notes.",
        point_delta: 39,
        created_at: "2026-08-10T10:08:00Z",
      },
      {
        task_id: "t1",
        judge_slug: "ux-review",
        judge_name: "UX Review",
        rating: 7,
        feedback: "Cards read well on desktop; mobile spacing is cramped below 400px.",
        point_delta: 18,
        created_at: "2026-08-10T10:09:00Z",
      },
    ],
  ],
]);

const judgeStatusesByTask = new Map([
  [
    "t2",
    [
      {
        task_id: "t2",
        judge_slug: "correctness",
        judge_name: "Correctness",
        status: "running",
      },
    ],
  ],
]);

const evaluationsByTask = new Map([
  [
    "t1",
    {
      task_id: "t1",
      criteria: [],
      todo: {
        checked: 3,
        total: 3,
        items: [
          { text: "Fetch the forecast dataset", done: true },
          { text: "Render the city cards", done: true },
          { text: "Write the done-file", done: true },
        ],
      },
    },
  ],
  [
    "t2",
    {
      task_id: "t2",
      criteria: [],
      pending_artifacts: [
        {
          probe_id: "a1",
          instruction:
            "Record a short screencast of the running widget: switch through every city, then open Rome's 3-day forecast.",
          path: ".ololo/artifacts/a1/",
          deadline_at: new Date(Date.now() + 4 * 60_000).toISOString(),
        },
      ],
    },
  ],
]);

const changesByTask = new Map([
  [
    "t1",
    {
      mode: "per-commit" as const,
      commits: [
        {
          sha: "ab12cd34ef56",
          author_name: "player",
          author_email: "p@example.com",
          author_time: "2026-08-10T10:03:00Z",
          message: "feat(t1): weather widget with three city cards",
          files: [
            { path: "index.html", status: "added", patch: "" },
            { path: "widget.js", status: "added", patch: "" },
          ],
        },
      ],
    },
  ],
]);

const meta = {
  title: "Sessions/PlayerTaskChat",
  component: PlayerTaskChat,
  parameters: { layout: "padded", backgrounds: { default: "light-blue" } },
} satisfies Meta<typeof PlayerTaskChat>;

export default meta;
type Story = StoryObj<typeof meta>;

export const LiveSession: Story = {
  args: {
    tasks,
    probesByTask,
    judgeResultsByTask,
    judgeStatusesByTask,
    evaluationsByTask,
    changesByTask,
    judgeAvatars: {},
    sessionCode: "ABC123",
    playerId: "player-1",
    playerName: "Kai",
    avatarUrl: null,
    agentDisplayName: "Claude Code",
    sessionFinished: false,
    live: true,
  },
};

export const FinishedSession: Story = {
  args: {
    ...LiveSession.args,
    judgeStatusesByTask: new Map(),
    sessionFinished: true,
    live: false,
  },
};

/** The status footer counting down to the next check while the previous
 *  task's judges are still reading — the live session's resting state. */
export const NextCheckCountdown: Story = {
  args: {
    ...LiveSession.args,
    nextProbeAt: new Date(Date.now() + 45_000).toISOString(),
    agentConnected: true,
    completionStatus: "in_progress",
  },
};

export const AgentDisconnected: Story = {
  args: {
    ...LiveSession.args,
    agentConnected: false,
  },
};
