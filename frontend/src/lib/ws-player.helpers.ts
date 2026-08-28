import { WsPlayerClient } from "$lib/ws-player.svelte";
import type {
  PlayerSnapshotPayload,
  PlayerProbeEntry,
  PlayerTaskSummaryEntry,
  ProbeUpdatedPayload,
} from "$lib/types/arena.js";

export function makeSnapshot(): PlayerSnapshotPayload {
  return {
    player_id: "player-1",
    display_name: "Test",
    score: 0,
    rank: 1,
    last_seq: 0,
    probes: [],
    tasks: [],
    next_probe_at: null,
    total_tasks: 0,
    session_started_at: null,
    session_ends_at: null,
    session_status: "running",
  };
}

export function makeProbeUpdated(
  overrides: Partial<ProbeUpdatedPayload> = {},
): ProbeUpdatedPayload {
  return {
    seq: 1,
    probe_id: "probe-1",
    task_id: "t1",
    task_title: "Task One",
    task_ordinal: 1,
    adapted_test_id: "test-1",
    test_command: "run test",
    attempt: 1,
    rendered_command: "run test",
    fixture_values: null,
    expected_answer: "42",
    state: "dispatched",
    outcome: null,
    actual: null,
    expected: null,
    exit_code: null,
    duration_ms: null,
    dispatched_at: "2024-01-01T00:00:00Z",
    deadline_at: "2024-01-01T00:00:30Z",
    resolved_at: null,
    point_delta: 0,
    score: 10,
    rank: 2,
    updated_at: "2024-01-01T00:00:00Z",
    next_probe_at: null,
    ...overrides,
  };
}

/**
 * In-memory WebSocket stand-in. Mirrors the subset of the browser `WebSocket`
 * API the client relies on (connect callback, message dispatch, send/close).
 */
export class FakeWebSocket {
  static lastInstance: FakeWebSocket | null = null;
  listeners: Record<string, EventListener[]> = {};
  sent: string[] = [];
  readyState = 0;
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;

  constructor() {
    FakeWebSocket.lastInstance = this;
  }

  addEventListener(type: string, listener: EventListener) {
    if (!this.listeners[type]) this.listeners[type] = [];
    this.listeners[type].push(listener);
  }

  removeEventListener() {}

  emit(type: string, data?: unknown) {
    if (type === "message") {
      const payload = (data as { data?: unknown } | null)?.data ?? data;
      this.onmessage?.(new MessageEvent("message", { data: payload as string }));
    } else if (type === "open") {
      this.onopen?.(new Event("open"));
    } else if (type === "close") {
      this.onclose?.({} as CloseEvent);
    } else if (type === "error") {
      this.onerror?.(new Event("error"));
    }
    for (const l of this.listeners[type] ?? []) {
      if (typeof data === "object" && data !== null && "data" in (data as object)) {
        l(new MessageEvent(type, data as MessageEventInit));
      } else {
        l(new Event(type));
      }
    }
  }

  send(data: string) {
    this.sent.push(data);
  }

  close() {
    this.readyState = 3;
    this.onclose?.({} as CloseEvent);
  }
}

export function makeProbeEntry(overrides: Partial<PlayerProbeEntry> = {}): PlayerProbeEntry {
  return {
    id: "p1",
    task_id: "t1",
    task_title: "Task One",
    task_ordinal: 1,
    adapted_test_id: "at1",
    test_command: "echo",
    attempt: 1,
    rendered_command: "echo",
    fixture_values: null,
    expected_answer: "hi",
    state: "resolved",
    outcome: "pass",
    actual: "hi",
    expected: "hi",
    exit_code: 0,
    duration_ms: 10,
    dispatched_at: "2026-01-01T00:00:00Z",
    deadline_at: null,
    resolved_at: "2026-01-01T00:00:01Z",
    result: { status: "passed", expected: "hi", actual: "hi" },
    point_delta: 10,
    updated_at: "2026-01-01T00:00:01Z",
    ...overrides,
  };
}

export function makeTaskSummary(
  overrides: Partial<PlayerTaskSummaryEntry> = {},
): PlayerTaskSummaryEntry {
  return {
    task_id: "t1",
    ordinal: 1,
    title: "Task One",
    content: "do",
    tags: [],
    adapted_content: "",
    result: null,
    scheduler_state: { state: "active", activated_at: null, deadline_at: null },
    ...overrides,
  };
}

/** Serialize a frame and push it through the socket's message handler. */
export function emitFrame(ws: FakeWebSocket, frame: unknown): void {
  ws.emit("message", { data: JSON.stringify(frame) });
}

// @ts-expect-error mock
globalThis.WebSocket = FakeWebSocket;

/** Construct a client, connect, and fire the open handshake. */
export function connectClient(
  playerId = "player-1",
  session = "SESSION",
  token = "token",
): { client: WsPlayerClient; ws: FakeWebSocket } {
  const client = new WsPlayerClient(playerId, session, token);
  client.connect();
  const ws = FakeWebSocket.lastInstance!;
  ws.emit("open");
  return { client, ws };
}
