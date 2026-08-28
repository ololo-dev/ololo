import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ArenaFrame } from "$lib/types/arena";

const captured: { path: string; onMessage: (raw: string) => void }[] = [];

vi.mock("$lib/ws/connection.svelte", () => ({
  UNBOUNDED_RECONNECT: { initialMs: 1_000, maxMs: 30_000, multiplier: 2, maxAttempts: Infinity },
  createFrameConnection: vi.fn((opts: { path: string; onFrame: (frame: unknown) => void }) => {
    // Mirror the real helper's parse-and-drop behavior so tests can feed
    // raw strings (including malformed JSON) through `onMessage`.
    captured.push({
      path: opts.path,
      onMessage: (raw: string) => {
        let frame: unknown;
        try {
          frame = JSON.parse(raw);
        } catch {
          return;
        }
        opts.onFrame(frame);
      },
    });
    return { connect: vi.fn(), disconnect: vi.fn(), connected: false };
  }),
}));

import { WsProjectClient } from "./ws-project.svelte";

function makeClient(onUpdate = vi.fn()) {
  const client = new WsProjectClient("proj1", onUpdate);
  return { client, onUpdate };
}

describe("WsProjectClient", () => {
  beforeEach(() => {
    captured.length = 0;
  });

  it("builds the observe path and delegates connect/disconnect", () => {
    const { client } = makeClient();
    client.connect();
    client.disconnect();
    expect(captured[0].path).toBe("/ws/project/proj1/observe");
  });

  it("exposes connected from the underlying connection", () => {
    const { client } = makeClient();
    expect(client.connected).toBe(false);
  });

  it("ignores malformed JSON", () => {
    const { client } = makeClient();
    expect(() => captured[0].onMessage("not json")).not.toThrow();
    expect(client.sessionCountdowns).toEqual({});
  });

  it("forwards project_session_update frames to the callback", () => {
    const { client, onUpdate } = makeClient();
    const frame = {
      type: "project_session_update",
      session_id: "s1",
      status: "running",
    } as ArenaFrame;
    captured[0].onMessage(JSON.stringify(frame));
    expect(onUpdate).toHaveBeenCalledWith(frame);
  });

  it("clears a countdown when a session reaches a terminal state", () => {
    const { client } = makeClient();
    captured[0].onMessage(
      JSON.stringify({ type: "lobby_countdown", session_id: "s1", seconds_remaining: 5 }),
    );
    expect(client.sessionCountdowns.s1).toEqual({ type: "lobby", secs: 5 });
    captured[0].onMessage(
      JSON.stringify({ type: "project_session_update", session_id: "s1", status: "finished" }),
    );
    expect(client.sessionCountdowns.s1).toBeUndefined();
  });

  it("tracks lobby and running countdowns", () => {
    const { client } = makeClient();
    captured[0].onMessage(
      JSON.stringify({ type: "lobby_countdown", session_id: "s1", seconds_remaining: 9 }),
    );
    captured[0].onMessage(
      JSON.stringify({ type: "running_countdown", session_id: "s2", seconds_remaining: 3 }),
    );
    expect(client.sessionCountdowns).toEqual({
      s1: { type: "lobby", secs: 9 },
      s2: { type: "running", secs: 3 },
    });
  });

  it("ignores unknown frame types", () => {
    const { client } = makeClient();
    captured[0].onMessage(JSON.stringify({ type: "something_else" }));
    expect(client.sessionCountdowns).toEqual({});
  });
});
