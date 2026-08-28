import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createWsConnection } from "./connection.svelte";

class MockWebSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;
  readyState = MockWebSocket.OPEN;
  url: string;
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  static instances: MockWebSocket[] = [];

  constructor(url: string) {
    this.url = url;
    MockWebSocket.instances.push(this);
  }
  close() {
    this.readyState = MockWebSocket.CLOSED;
    this.onclose?.({} as CloseEvent);
  }
  send(_data: string) {}
}

describe("createWsConnection", () => {
  beforeEach(() => {
    MockWebSocket.instances = [];
    vi.stubGlobal("WebSocket", MockWebSocket as unknown as typeof WebSocket);
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("invokes onMessage with the raw string payload", () => {
    const onMessage = vi.fn();
    const conn = createWsConnection({ path: "/ws/s/ABC", onMessage });
    conn.connect();

    MockWebSocket.instances[0].onmessage?.({ data: '{"type":"heartbeat"}' } as MessageEvent);

    expect(onMessage).toHaveBeenCalledExactlyOnceWith('{"type":"heartbeat"}');
    conn.disconnect();
  });

  it("reconnects with exponential backoff up to maxAttempts", () => {
    vi.useFakeTimers();
    const onMessage = vi.fn();
    const setTimeoutSpy = vi.spyOn(globalThis, "setTimeout");

    const conn = createWsConnection({
      path: "/ws/s/ABC",
      onMessage,
      reconnect: { initialMs: 100, maxMs: 1000, multiplier: 2, maxAttempts: 3 },
    });
    conn.connect();

    expect(MockWebSocket.instances).toHaveLength(1);

    MockWebSocket.instances[0].close();
    vi.advanceTimersByTime(100);
    MockWebSocket.instances[1].close();
    vi.advanceTimersByTime(200);
    MockWebSocket.instances[2].close();
    vi.advanceTimersByTime(400);
    MockWebSocket.instances[3].close();

    // 1 initial connect + 3 reconnects == maxAttempts
    expect(MockWebSocket.instances).toHaveLength(4);

    const delays = setTimeoutSpy.mock.calls.map((c) => c[1] as number);
    expect(delays).toEqual([100, 200, 400]);

    conn.disconnect();
  });

  it("stops reconnecting after disconnect", () => {
    vi.useFakeTimers();
    const conn = createWsConnection({
      path: "/x",
      onMessage: vi.fn(),
      reconnect: { initialMs: 100, maxMs: 1000, multiplier: 2, maxAttempts: 3 },
    });
    conn.connect();

    MockWebSocket.instances[0].close();
    conn.disconnect();
    vi.advanceTimersByTime(1000);

    expect(MockWebSocket.instances).toHaveLength(1);
  });
});
