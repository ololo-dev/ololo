import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render } from "@testing-library/svelte";
import type {
  PlayerSnapshotPayload,
  PlayerTaskSummaryEntry,
  PlayerProbeEntry,
  PlayerHistoryResponse,
  TaskStatsResponse,
} from "$lib/types/arena";
import Page from "./+page.svelte";

vi.mock("$lib/ws-player.svelte", () => ({
  WsPlayerClient: class {
    connect = vi.fn();
    disconnect = vi.fn();
    connected = false;
    snapshot = null;
    sessionFinished = false;
    error = null;
    judgeResults: never[] = [];
    constructor() {}
  },
}));

vi.mock("$app/environment", () => ({ browser: true }));
vi.mock("$app/state", () => ({
  page: { params: { code: "TEST01", player_id: "player-1" } },
}));

export type TestData = {
  live: boolean;
  judgesSettling?: boolean;
  snapshot: PlayerSnapshotPayload;
  token: string | null;
  playerName: string;
  history?: PlayerHistoryResponse | null;
  /** The server's 403 boundary: a spectator reads the run, not its diffs. */
  inspectRestricted?: boolean;
  taskStats?: TaskStatsResponse | null;
  isAuthenticated?: boolean;
  isAdmin?: boolean;
  /** The instance-wide replay switch; the bar needs it and admin both. */
  replayEnabled?: boolean;
  allowProjectCreation?: boolean;
  user?: {
    id: string;
    name: string;
    initials: string;
    avatarUrl: string | undefined;
    username: string;
  } | null;
};

export function makeSnapshot(
  overrides: Partial<PlayerSnapshotPayload> = {},
): PlayerSnapshotPayload {
  return {
    player_id: "player-1",
    display_name: "Test Player",
    score: 42,
    rank: 3,
    last_seq: 0,
    probes: [],
    tasks: [],
    next_probe_at: null,
    total_tasks: 0,
    session_started_at: null,
    session_ends_at: null,
    session_status: "running",
    ...overrides,
  };
}

export function makeTaskSummary(
  overrides: Partial<PlayerTaskSummaryEntry> = {},
): PlayerTaskSummaryEntry {
  return {
    task_id: "task-1",
    ordinal: 1,
    title: "Task One",
    content: "Task content",
    tags: [],
    adapted_content: "Write a hello world function",
    result: null,
    scheduler_state: null,
    ...overrides,
  };
}

export function makeProbe(overrides: Partial<PlayerProbeEntry> = {}): PlayerProbeEntry {
  return {
    id: "probe-1",
    task_id: "task-1",
    task_title: "Task One",
    task_ordinal: 1,
    adapted_test_id: "test-1",
    test_command: "run test",
    attempt: 1,
    rendered_command: "",
    fixture_values: null,
    expected_answer: null,
    state: "dispatched",
    outcome: null,
    actual: null,
    expected: null,
    exit_code: null,
    duration_ms: null,
    dispatched_at: null,
    deadline_at: null,
    resolved_at: null,
    result: null,
    point_delta: 0,
    updated_at: null,
    ...overrides,
  };
}

export function renderPage(data: TestData) {
  return render(Page, {
    props: {
      data: {
        isAuthenticated: false,
        isAdmin: false,
        replayEnabled: true,
        allowProjectCreation: false,
        user: null,
        history: null,
        inspectRestricted: false,
        taskStats: null,
        judgesSettling: false,
        judgeAvatars: {},
        sessionId: null,
        ...data,
      },
    },
  });
}
