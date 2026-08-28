/**
 * Generic WebSocket transport with exponential-backoff reconnect.
 * Owns the raw socket lifecycle; per-client typed dispatch lives in the
 * callers. Browser-only: connect() is a no-op outside a browser context.
 */

export interface ReconnectConfig {
  initialMs: number;
  maxMs: number;
  multiplier: number;
  maxAttempts: number;
}

export interface WsConnectionOptions {
  /** Path appended to the derived base URL, e.g. "/ws/s/ABC". */
  path: string;
  onMessage: (raw: string) => void;
  /** Partial override of the default backoff config. */
  reconnect?: Partial<ReconnectConfig>;
}

export interface WsConnection {
  connect(): void;
  disconnect(): void;
  readonly connected: boolean;
}

const DEFAULT_RECONNECT: ReconnectConfig = {
  initialMs: 1_000,
  maxMs: 30_000,
  multiplier: 2,
  maxAttempts: 8,
};

/** Reconnect forever — for passive observer dashboards that must self-heal. */
export const UNBOUNDED_RECONNECT: ReconnectConfig = {
  initialMs: 1_000,
  maxMs: 30_000,
  multiplier: 2,
  maxAttempts: Infinity,
};

function deriveBaseUrl(): string {
  if (typeof window === "undefined") return "";
  const proto = window.location.protocol === "https:" ? "wss" : "ws";
  return `${proto}://${window.location.host}`;
}

export function createWsConnection(opts: WsConnectionOptions): WsConnection {
  const reconnect = { ...DEFAULT_RECONNECT, ...opts.reconnect };

  let ws: WebSocket | null = null;
  let destroyed = false;
  let attempts = 0;
  let nextDelay = reconnect.initialMs;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let connected = $state(false);

  function open() {
    if (destroyed || ws) return;

    const url = `${deriveBaseUrl()}${opts.path}`;
    const socket = new WebSocket(url);
    ws = socket;

    socket.onopen = () => {
      if (destroyed) {
        socket.close();
        return;
      }
      connected = true;
      attempts = 0;
      nextDelay = reconnect.initialMs;
    };

    socket.onmessage = (ev) => {
      if (typeof ev.data === "string") opts.onMessage(ev.data);
    };

    socket.onclose = () => {
      connected = false;
      ws = null;
      if (!destroyed) scheduleReconnect();
    };

    socket.onerror = () => {
      // onclose fires right after onerror; reconnect is handled there.
    };
  }

  function scheduleReconnect() {
    if (destroyed || attempts >= reconnect.maxAttempts) return;

    attempts += 1;
    const delay = nextDelay;
    nextDelay = Math.min(nextDelay * reconnect.multiplier, reconnect.maxMs);
    timer = setTimeout(() => {
      timer = null;
      open();
    }, delay);
  }

  function connect() {
    destroyed = false;
    if (typeof window === "undefined") return;
    open();
  }

  function disconnect() {
    destroyed = true;
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    ws?.close();
    ws = null;
    connected = false;
  }

  return {
    connect,
    disconnect,
    get connected() {
      return connected;
    },
  };
}

export interface FrameConnectionOptions<F> {
  /** Path appended to the derived base URL, e.g. "/ws/s/ABC". */
  path: string;
  /** Called with each JSON-parsed frame; malformed messages are dropped. */
  onFrame: (frame: F) => void;
  /** Partial override of the default backoff config. */
  reconnect?: Partial<ReconnectConfig>;
}

/**
 * `createWsConnection` plus the JSON-parse-and-dispatch step shared by all
 * typed frame clients.
 */
export function createFrameConnection<F>(opts: FrameConnectionOptions<F>): WsConnection {
  return createWsConnection({
    path: opts.path,
    reconnect: opts.reconnect,
    onMessage: (raw) => {
      let frame: F;
      try {
        frame = JSON.parse(raw) as F;
      } catch {
        return;
      }
      opts.onFrame(frame);
    },
  });
}
