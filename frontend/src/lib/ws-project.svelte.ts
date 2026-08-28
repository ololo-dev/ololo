/**
 * Typed WebSocket client for the project observer endpoint.
 * Connects to `GET /ws/project/:project_id/observe` — no credentials required.
 *
 * Usage:
 *   const client = new WsProjectClient(projectId, (frame) => { ... });
 *   client.connect();
 *   // frame.type === "project_session_update" on every session change
 *   // client.sessionCountdowns[sessionId] gives live ticker data
 *   client.disconnect();
 *
 * Transport (open/close/backoff) is delegated to `createWsConnection`.
 */

import type { ArenaFrame } from "$lib/types/arena";
import {
  UNBOUNDED_RECONNECT,
  createFrameConnection,
  type WsConnection,
} from "$lib/ws/connection.svelte";

export type ProjectSessionUpdateFrame = Extract<ArenaFrame, { type: "project_session_update" }>;

export interface SessionCountdown {
  type: "lobby" | "running";
  secs: number;
}

export class WsProjectClient {
  private readonly projectId: string;
  private readonly serverBase: string;
  private readonly onUpdate: (frame: ProjectSessionUpdateFrame) => void;
  private readonly _conn: WsConnection;

  // --- reactive state ---
  get connected() {
    return this._conn.connected;
  }
  /** Live per-session countdown keyed by session_id (DB id). */
  sessionCountdowns = $state<Record<string, SessionCountdown>>({});

  constructor(
    projectId: string,
    onUpdate: (frame: ProjectSessionUpdateFrame) => void,
    serverBase = "",
  ) {
    this.projectId = projectId;
    this.onUpdate = onUpdate;
    this.serverBase = serverBase;

    const path = `/ws/project/${projectId}/observe`;

    this._conn = createFrameConnection<ArenaFrame>({
      path,
      onFrame: (frame) => this._handleFrame(frame),
      reconnect: UNBOUNDED_RECONNECT,
    });
  }

  connect(): void {
    this._conn.connect();
  }

  disconnect(): void {
    this._conn.disconnect();
  }

  private _handleFrame(frame: ArenaFrame): void {
    switch (frame.type) {
      case "project_session_update":
        this.onUpdate(frame);
        // Clear ticker when session reaches a terminal state.
        if (frame.status === "finished" || frame.status === "cancelled") {
          const { [frame.session_id]: _removed, ...rest } = this.sessionCountdowns;
          this.sessionCountdowns = rest;
        }
        break;

      case "lobby_countdown":
        this.sessionCountdowns = {
          ...this.sessionCountdowns,
          [frame.session_id]: { type: "lobby", secs: frame.seconds_remaining },
        };
        break;

      case "running_countdown":
        this.sessionCountdowns = {
          ...this.sessionCountdowns,
          [frame.session_id]: { type: "running", secs: frame.seconds_remaining },
        };
        break;

      default:
        break;
    }
  }
}
