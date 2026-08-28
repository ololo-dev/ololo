import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { WsPlayerClient } from "$lib/ws-player.svelte";
import type { PlayerFrame } from "$lib/types/arena.js";
import {
  connectClient,
  makeSnapshot,
  makeProbeUpdated,
  makeProbeEntry,
  makeTaskSummary,
  emitFrame,
  FakeWebSocket,
} from "$lib/ws-player.helpers";

describe("ws-player test helpers", () => {
  it("makeSnapshot builds a valid running snapshot", () => {
    const s = makeSnapshot();
    expect(s.player_id).toBe("player-1");
    expect(s.session_status).toBe("running");
    expect(s.tasks).toEqual([]);
    expect(s.probes).toEqual([]);
  });

  it("makeProbeUpdated applies overrides without dropping defaults", () => {
    const p = makeProbeUpdated({ state: "resolved", score: 99, outcome: "passed" });
    expect(p.state).toBe("resolved");
    expect(p.score).toBe(99);
    expect(p.outcome).toBe("passed");
    expect(p.probe_id).toBe("probe-1");
    expect(p.task_id).toBe("t1");
  });

  it("FakeWebSocket delivers a parsed message to onmessage", () => {
    const ws = new FakeWebSocket();
    let received: unknown = null;
    ws.onmessage = (e) => {
      received = (e as MessageEvent).data;
    };
    ws.emit("message", { data: "hello" });
    expect(received).toBe("hello");
  });

  it("connectClient returns a connected client and live socket", () => {
    const { client, ws } = connectClient();
    expect(client.connected).toBe(true);
    expect(ws).toBeInstanceOf(FakeWebSocket);
  });
});

describe("WsPlayerClient", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    FakeWebSocket.lastInstance = null;
  });

  function open() {
    const { client, ws } = connectClient();
    return { client, ws };
  }

  it("connects and sets connected on open", () => {
    const { client } = open();
    expect(client.connected).toBe(true);
  });

  it("applies player_snapshot frame to snapshot state", () => {
    const { client, ws } = open();
    emitFrame(ws, { type: "player_snapshot", ...makeSnapshot() } satisfies PlayerFrame);
    expect(client.snapshot).not.toBeNull();
    expect(client.snapshot!.score).toBe(0);
    expect(client.snapshot!.last_seq).toBe(0);
  });

  it("exposes completion_status from player_snapshot when present", () => {
    const { client, ws } = open();
    emitFrame(ws, {
      type: "player_snapshot",
      ...makeSnapshot(),
      completion_status: "completed",
    } satisfies PlayerFrame);
    expect(client.snapshot!.completion_status).toBe("completed");
  });

  it("leaves completion_status undefined when absent from player_snapshot", () => {
    const { client, ws } = open();
    emitFrame(ws, { type: "player_snapshot", ...makeSnapshot() } satisfies PlayerFrame);
    expect(client.snapshot!.completion_status).toBeUndefined();
  });

  it("applies player_task_delta to existing task", () => {
    const { client, ws } = open();
    emitFrame(ws, {
      type: "player_snapshot",
      ...makeSnapshot(),
      tasks: [makeTaskSummary({ result: null, scheduler_state: null })],
    });
    emitFrame(ws, {
      type: "player_task_delta",
      seq: 1,
      task_id: "t1",
      result: null,
      scheduler_state: { state: "backoff", activated_at: null, deadline_at: null },
      score: 10,
      rank: 2,
    } satisfies PlayerFrame);
    const task = client.snapshot!.tasks.find((t) => t.task_id === "t1");
    expect(task?.scheduler_state?.state).toBe("backoff");
    expect(client.snapshot!.score).toBe(10);
    expect(client.snapshot!.rank).toBe(2);
  });

  it("clears scheduler_state when a player_task_delta carries a result", () => {
    // The advance delta means the scheduler let go of the task: its null
    // scheduler_state is "cleared", not "unchanged" — otherwise a stale
    // "judging" hold would outlive the task on live pages.
    const { client, ws } = open();
    emitFrame(ws, {
      type: "player_snapshot",
      ...makeSnapshot(),
      tasks: [
        makeTaskSummary({
          result: null,
          scheduler_state: { state: "judging", activated_at: null, deadline_at: null },
        }),
      ],
    });
    emitFrame(ws, {
      type: "player_task_delta",
      seq: 1,
      task_id: "t1",
      result: {
        status: "completed",
        submitted_answer: null,
        correct_answer: null,
        score_delta: 0,
        evaluated_at: null,
      },
      scheduler_state: null,
      score: 10,
      rank: 1,
    } satisfies PlayerFrame);
    const task = client.snapshot!.tasks.find((t) => t.task_id === "t1");
    expect(task?.result?.status).toBe("completed");
    expect(task?.scheduler_state).toBeNull();
  });

  it("keeps scheduler_state on a result-less player_task_delta with null state", () => {
    const { client, ws } = open();
    emitFrame(ws, {
      type: "player_snapshot",
      ...makeSnapshot(),
      tasks: [
        makeTaskSummary({
          result: null,
          scheduler_state: { state: "active", activated_at: null, deadline_at: null },
        }),
      ],
    });
    emitFrame(ws, {
      type: "player_task_delta",
      seq: 1,
      task_id: "t1",
      result: null,
      scheduler_state: null,
      score: 0,
      rank: 1,
    } satisfies PlayerFrame);
    const task = client.snapshot!.tasks.find((t) => t.task_id === "t1");
    expect(task?.scheduler_state?.state).toBe("active");
  });

  it("sets error on player_error frame", () => {
    const { client, ws } = open();
    emitFrame(ws, { type: "player_error", seq: 1, message: "oops" });
    expect(client.error).toBe("oops");
  });

  it("sets sessionFinished on session_complete frame", () => {
    const { client, ws } = open();
    emitFrame(ws, { type: "session_complete", seq: 5, player_id: "player-1", score: 100, rank: 1 });
    expect(client.sessionFinished).toBe(true);
  });

  it("discards frames with seq <= last_seq", () => {
    const { client, ws } = open();
    emitFrame(ws, { type: "player_snapshot", ...makeSnapshot(), last_seq: 3 });
    emitFrame(ws, {
      type: "player_task_delta",
      seq: 2,
      task_id: "t1",
      result: null,
      scheduler_state: { state: "backoff", activated_at: null, deadline_at: null },
      score: 10,
      rank: 2,
    } satisfies PlayerFrame);
    expect(client.snapshot!.score).toBe(0);
  });

  it("upserts probe on probe_updated with state dispatched", () => {
    const { client, ws } = open();
    emitFrame(ws, { type: "player_snapshot", ...makeSnapshot() });
    emitFrame(ws, { type: "probe_updated", ...makeProbeUpdated() } satisfies PlayerFrame);
    const probe = client.snapshot!.probes.find((p) => p.id === "probe-1");
    expect(probe).toBeDefined();
    expect(probe!.state).toBe("dispatched");
    expect(probe!.outcome).toBeNull();
    expect(client.snapshot!.score).toBe(10);
    expect(client.snapshot!.rank).toBe(2);
  });

  it("updates probe outcome on probe_updated with state resolved", () => {
    const { client, ws } = open();
    emitFrame(ws, { type: "player_snapshot", ...makeSnapshot() });
    emitFrame(ws, { type: "probe_updated", ...makeProbeUpdated() });
    emitFrame(ws, {
      type: "probe_updated",
      ...makeProbeUpdated({
        seq: 2,
        state: "resolved",
        outcome: "passed",
        actual: "42",
        expected: "42",
        exit_code: 0,
        duration_ms: 500,
        resolved_at: "2024-01-01T00:00:15Z",
        point_delta: 5,
        score: 15,
        rank: 1,
      }),
    } satisfies PlayerFrame);
    const probe = client.snapshot!.probes.find((p) => p.id === "probe-1");
    expect(probe!.state).toBe("resolved");
    expect(probe!.outcome).toBe("passed");
    expect(probe!.result).toEqual({ status: "passed", expected: "42", actual: "42" });
    expect(client.snapshot!.score).toBe(15);
    expect(client.snapshot!.rank).toBe(1);
  });

  it("replaces existing probe on probe_updated (immutable update)", () => {
    const { client, ws } = open();
    emitFrame(ws, { type: "player_snapshot", ...makeSnapshot() });
    emitFrame(ws, { type: "probe_updated", ...makeProbeUpdated() });
    expect(client.snapshot!.probes[0].dispatched_at).toBe("2024-01-01T00:00:00Z");

    emitFrame(ws, {
      type: "probe_updated",
      ...makeProbeUpdated({
        seq: 2,
        state: "resolved",
        outcome: "failed",
        actual: "0",
        expected: "42",
        exit_code: 1,
        duration_ms: 300,
        resolved_at: "2024-01-01T00:00:10Z",
        point_delta: -3,
        score: 7,
        rank: 4,
        dispatched_at: "2024-01-01T00:00:00Z",
      }),
    } satisfies PlayerFrame);

    expect(client.snapshot!.probes.length).toBe(1);
    const probe1 = client.snapshot!.probes[0];
    expect(probe1.state).toBe("resolved");
    expect(probe1.outcome).toBe("failed");
    expect(probe1.dispatched_at).toBe("2024-01-01T00:00:00Z");
    expect(probe1.resolved_at).toBe("2024-01-01T00:00:10Z");
    expect(probe1.point_delta).toBe(-3);
    expect(client.snapshot!.score).toBe(7);
    expect(client.snapshot!.rank).toBe(4);
  });

  it("_upsertProbe creates stub task when task not in snapshot", () => {
    const { client, ws } = open();
    emitFrame(ws, { type: "player_snapshot", ...makeSnapshot(), tasks: [] });
    emitFrame(ws, {
      type: "probe_updated",
      ...makeProbeUpdated({ task_id: "unknown-task", task_title: "New Task", task_ordinal: 3 }),
    } satisfies PlayerFrame);
    const task = client.snapshot!.tasks.find((t) => t.task_id === "unknown-task");
    expect(task).toBeDefined();
    expect(task!.title).toBe("New Task");
    expect(task!.ordinal).toBe(3);
    expect(task!.content).toBe("");
    expect(task!.tags).toEqual([]);
    expect(task!.result).toBeNull();
  });

  it("updates score and rank from probe_updated", () => {
    const { client, ws } = open();
    emitFrame(ws, { type: "player_snapshot", ...makeSnapshot() });
    emitFrame(ws, { type: "probe_updated", ...makeProbeUpdated({ score: 42, rank: 1 }) });
    expect(client.snapshot!.score).toBe(42);
    expect(client.snapshot!.rank).toBe(1);
    emitFrame(ws, { type: "probe_updated", ...makeProbeUpdated({ seq: 2, score: 55, rank: 2 }) });
    expect(client.snapshot!.score).toBe(55);
    expect(client.snapshot!.rank).toBe(2);
  });

  it("inserts task on task_revealed when task not in snapshot", () => {
    const { client, ws } = open();
    emitFrame(ws, { type: "player_snapshot", ...makeSnapshot(), total_tasks: 3 });
    emitFrame(ws, {
      type: "task_revealed",
      seq: 1,
      task: makeTaskSummary({
        task_id: "t1",
        ordinal: 1,
        title: "Task One",
        content: "Do stuff",
        tags: ["easy"],
        adapted_content: "adapted",
      }),
      total_tasks: 5,
    });
    expect(client.snapshot!.tasks.length).toBe(1);
    expect(client.snapshot!.tasks[0].task_id).toBe("t1");
    expect(client.snapshot!.tasks[0].title).toBe("Task One");
    expect(client.snapshot!.tasks[0].content).toBe("Do stuff");
    expect(client.snapshot!.tasks[0].tags).toEqual(["easy"]);
    expect(client.snapshot!.total_tasks).toBe(5);
  });

  it("replaces task on task_revealed when task already exists", () => {
    const { client, ws } = open();
    emitFrame(ws, {
      type: "player_snapshot",
      ...makeSnapshot(),
      tasks: [
        makeTaskSummary({ title: "Old Title", content: "old", tags: [], adapted_content: "" }),
      ],
      total_tasks: 3,
    });
    emitFrame(ws, {
      type: "task_revealed",
      seq: 1,
      task: makeTaskSummary({
        title: "New Title",
        content: "new",
        tags: ["updated"],
        adapted_content: "new adapted",
      }),
      total_tasks: 4,
    });
    expect(client.snapshot!.tasks.length).toBe(1);
    expect(client.snapshot!.tasks[0].task_id).toBe("t1");
    expect(client.snapshot!.tasks[0].title).toBe("New Title");
    expect(client.snapshot!.tasks[0].content).toBe("new");
    expect(client.snapshot!.tasks[0].tags).toEqual(["updated"]);
    expect(client.snapshot!.total_tasks).toBe(4);
  });

  it("marks previous active task as completed on task_revealed (all probes passed)", () => {
    const { client, ws } = open();
    emitFrame(ws, {
      type: "player_snapshot",
      ...makeSnapshot(),
      tasks: [makeTaskSummary()],
      probes: [makeProbeEntry()],
      total_tasks: 2,
    });
    emitFrame(ws, {
      type: "task_revealed",
      seq: 1,
      task: makeTaskSummary({ task_id: "t2", ordinal: 2, title: "Task Two", content: "do2" }),
      total_tasks: 2,
    });
    expect(client.snapshot!.tasks.length).toBe(2);
    expect(client.snapshot!.tasks[0].task_id).toBe("t1");
    expect(client.snapshot!.tasks[0].result!.status).toBe("completed");
    expect(client.snapshot!.tasks[0].scheduler_state).toBeNull();
    expect(client.snapshot!.tasks[1].task_id).toBe("t2");
    expect(client.snapshot!.tasks[1].scheduler_state).not.toBeNull();
  });

  it("marks previous active task as failed on task_revealed (probe failed)", () => {
    const { client, ws } = open();
    emitFrame(ws, {
      type: "player_snapshot",
      ...makeSnapshot(),
      tasks: [makeTaskSummary()],
      probes: [
        makeProbeEntry({
          outcome: "error",
          actual: "no",
          result: { status: "failed", expected: "hi", actual: "no" },
          point_delta: -5,
        }),
      ],
      total_tasks: 2,
    });
    emitFrame(ws, {
      type: "task_revealed",
      seq: 1,
      task: makeTaskSummary({ task_id: "t2", ordinal: 2, title: "Task Two", content: "do2" }),
      total_tasks: 2,
    });
    expect(client.snapshot!.tasks[0].result!.status).toBe("failed");
  });

  it("discards task_revealed with seq <= last_seq", () => {
    const { client, ws } = open();
    emitFrame(ws, { type: "player_snapshot", ...makeSnapshot(), last_seq: 5, total_tasks: 3 });
    emitFrame(ws, {
      type: "task_revealed",
      seq: 3,
      task: makeTaskSummary({ task_id: "t1", ordinal: 1, title: "T" }),
      total_tasks: 5,
    });
    expect(client.snapshot!.tasks.length).toBe(0);
    expect(client.snapshot!.total_tasks).toBe(3);
  });

  it("updates score and rank on score_rank_updated", () => {
    const { client, ws } = open();
    emitFrame(ws, { type: "player_snapshot", ...makeSnapshot() });
    emitFrame(ws, { type: "score_rank_updated", seq: 1, score: 42, rank: 3 });
    expect(client.snapshot!.score).toBe(42);
    expect(client.snapshot!.rank).toBe(3);
  });

  it("discards score_rank_updated with seq <= last_seq", () => {
    const { client, ws } = open();
    emitFrame(ws, { type: "player_snapshot", ...makeSnapshot(), last_seq: 5 });
    emitFrame(ws, { type: "score_rank_updated", seq: 3, score: 99, rank: 1 });
    expect(client.snapshot!.score).toBe(0);
    expect(client.snapshot!.rank).toBe(1);
  });

  it("seeds judge statuses from the snapshot and advances them on lifecycle frames", () => {
    const { client, ws } = open();
    emitFrame(ws, {
      type: "player_snapshot",
      ...makeSnapshot(),
      judge_statuses: [
        {
          task_id: "t1",
          judge_slug: "code-cleanliness",
          judge_name: "Code Cleanliness",
          status: "pending",
          error: null,
          updated_at: null,
        },
      ],
    });
    expect(client.judgeStatuses.length).toBe(1);
    expect(client.judgeStatuses[0].status).toBe("pending");

    emitFrame(ws, {
      type: "judge_started",
      task_id: "t1",
      judge_slug: "code-cleanliness",
      judge_name: "Code Cleanliness",
      status: "running",
      error: null,
      updated_at: "2026-01-01T00:00:00Z",
    });
    expect(client.judgeStatuses.length).toBe(1);
    expect(client.judgeStatuses[0].status).toBe("running");

    emitFrame(ws, {
      type: "judge_failed",
      task_id: "t1",
      judge_slug: "code-cleanliness",
      judge_name: "Code Cleanliness",
      status: "failed",
      error: "ai_timeout",
      updated_at: "2026-01-01T00:00:09Z",
    });
    expect(client.judgeStatuses.length).toBe(1);
    expect(client.judgeStatuses[0].status).toBe("failed");
    expect(client.judgeStatuses[0].error).toBe("ai_timeout");
  });

  it("judge_scored marks the matching status entry as scored", () => {
    const { client, ws } = open();
    emitFrame(ws, {
      type: "player_snapshot",
      ...makeSnapshot(),
      tasks: [makeTaskSummary({ task_id: "t1", ordinal: 1, title: "T" })],
      judge_statuses: [
        {
          task_id: "t1",
          judge_slug: "code-cleanliness",
          judge_name: "Code Cleanliness",
          status: "running",
          error: null,
          updated_at: null,
        },
      ],
    });
    emitFrame(ws, {
      type: "judge_scored",
      task_id: "t1",
      judge_slug: "code-cleanliness",
      judge_name: "Code Cleanliness",
      rating: 4,
      feedback: "ok",
      point_delta: 8,
      created_at: "2026-01-01T00:00:10Z",
    });
    expect(client.judgeResults.length).toBe(1);
    expect(client.judgeStatuses.length).toBe(1);
    expect(client.judgeStatuses[0].status).toBe("scored");
  });
  it("captures the cancel reason and who cancelled from a status change", () => {
    const { client, ws } = open();
    emitFrame(ws, { type: "player_snapshot", ...makeSnapshot() } satisfies PlayerFrame);
    emitFrame(ws, {
      type: "session_status_change",
      status: "cancelled",
      cancel_reason: "user",
      cancelled_by: "Andrey",
    });
    expect(client.sessionFinished).toBe(true);
    expect(client.cancelReason).toBe("user");
    expect(client.cancelledBy).toBe("Andrey");
  });

  it("marks an idle-timeout cancellation without a canceller name", () => {
    const { client, ws } = open();
    emitFrame(ws, { type: "player_snapshot", ...makeSnapshot() } satisfies PlayerFrame);
    emitFrame(ws, {
      type: "session_status_change",
      status: "cancelled",
      cancel_reason: "idle_timeout",
    });
    expect(client.cancelReason).toBe("idle_timeout");
    expect(client.cancelledBy).toBeNull();
  });
});

describe("WsPlayerClient evaluation refresh", () => {
  const SNAPSHOT_URL = "/api/sessions/SESSION/players/player-1";
  let fetchMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.useFakeTimers();
    FakeWebSocket.lastInstance = null;
    fetchMock = vi.fn(() =>
      Promise.resolve({ json: () => Promise.resolve({}) } as unknown as Response),
    );
    vi.stubGlobal("fetch", fetchMock);
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  function open() {
    return connectClient();
  }

  it("re-fetches the snapshot on evaluation_ready", () => {
    const { ws } = open();
    emitFrame(ws, { type: "evaluation_ready", task_id: "t1" } satisfies PlayerFrame);
    vi.advanceTimersByTime(1);
    expect(fetchMock).toHaveBeenCalledWith(SNAPSHOT_URL);
  });

  it("re-fetches the snapshot when the session report lands", () => {
    // The report is written after the session finished, when the page's
    // reconcile poll has already stopped — the frame is what puts it on
    // screen without a reload.
    const { ws } = open();
    emitFrame(ws, { type: "session_report_ready" } satisfies PlayerFrame);
    vi.advanceTimersByTime(1);
    expect(fetchMock).toHaveBeenCalledWith(SNAPSHOT_URL);
  });

  it("re-fetches the snapshot on artifact_awaited", () => {
    const { ws } = open();
    emitFrame(ws, {
      type: "artifact_awaited",
      task_id: "t1",
      probe_id: "probe-1",
      instruction: "Take a screenshot",
      deadline_at: "2026-01-01T00:05:00Z",
    } satisfies PlayerFrame);
    vi.advanceTimersByTime(1);
    expect(fetchMock).toHaveBeenCalledWith(SNAPSHOT_URL);
  });

  it("coalesces a burst of judge_scored frames into one refresh", () => {
    const { ws } = open();
    emitFrame(ws, { type: "player_snapshot", ...makeSnapshot() } satisfies PlayerFrame);
    for (const slug of ["correctness", "architecture", "ux-review"]) {
      emitFrame(ws, {
        type: "judge_scored",
        task_id: "t1",
        judge_slug: slug,
        judge_name: slug,
        rating: 7,
        feedback: "ok",
        point_delta: 5,
        created_at: "2026-01-01T00:00:10Z",
      } satisfies PlayerFrame);
    }
    vi.advanceTimersByTime(1000);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock).toHaveBeenCalledWith(SNAPSHOT_URL);
  });

  it("re-fetches on judge_failed but not on judge_started", () => {
    const { ws } = open();
    emitFrame(ws, {
      type: "judge_started",
      task_id: "t1",
      judge_slug: "correctness",
      judge_name: "Correctness",
      status: "running",
      error: null,
      updated_at: "2026-01-01T00:00:00Z",
    } satisfies PlayerFrame);
    vi.advanceTimersByTime(1000);
    expect(fetchMock).not.toHaveBeenCalled();

    emitFrame(ws, {
      type: "judge_failed",
      task_id: "t1",
      judge_slug: "correctness",
      judge_name: "Correctness",
      status: "failed",
      error: "ai_timeout",
      updated_at: "2026-01-01T00:00:09Z",
    } satisfies PlayerFrame);
    vi.advanceTimersByTime(1000);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("disconnect cancels a pending refresh", () => {
    const { client, ws } = open();
    emitFrame(ws, {
      type: "judge_scored",
      task_id: "t1",
      judge_slug: "correctness",
      judge_name: "Correctness",
      rating: 7,
      feedback: "ok",
      point_delta: 5,
      created_at: "2026-01-01T00:00:10Z",
    } satisfies PlayerFrame);
    client.disconnect();
    vi.advanceTimersByTime(1000);
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
