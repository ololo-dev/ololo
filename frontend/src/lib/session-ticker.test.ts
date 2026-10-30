import { describe, expect, it } from "vitest";
import type { ActivityEvent } from "$lib/types/arena";
import { latestEvents, tickerLine } from "./session-ticker";

function ev(over: Partial<ActivityEvent>): ActivityEvent {
  return {
    kind: "task_started",
    player_id: "p1",
    player_display_name: "Alpha",
    task_id: "t1",
    task_ordinal: 2,
    task_title: "Aim phase",
    judge_name: null,
    point_delta: null,
    timestamp: "2026-10-29T10:00:00Z",
    version: 1,
    ...over,
  };
}

describe("tickerLine", () => {
  it("names the task a player started, with its title", () => {
    expect(tickerLine(ev({}))).toEqual({
      who: "Alpha",
      what: "started Task 2 · Aim phase",
      delta: null,
    });
  });

  it("names the judge and the points of a verdict", () => {
    expect(
      tickerLine(ev({ kind: "task_scored", judge_name: "UX Review", point_delta: 18 })),
    ).toEqual({
      who: "Alpha",
      what: "evaluated by UX Review on Task 2",
      delta: 18,
    });
  });

  it("reads a passed check as the task implemented", () => {
    expect(tickerLine(ev({ kind: "task_scored", point_delta: 25 }))).toEqual({
      who: "Alpha",
      what: "implemented Task 2",
      delta: 25,
    });
  });

  it("tells a copy/paste penalty from a clean check", () => {
    const dirty = ev({
      kind: "similarity",
      point_delta: -40,
      detail: { duplicated_pct: 37.4 } as never,
    });
    expect(tickerLine(dirty)).toEqual({
      who: "Alpha",
      what: "copy/paste check: 37% matched",
      delta: -40,
    });
    expect(tickerLine(ev({ kind: "similarity", point_delta: 0 })).what).toBe(
      "copy/paste check: clean",
    );
  });

  it("says a capture was delivered", () => {
    expect(tickerLine(ev({ kind: "artifact_received" })).what).toBe(
      "delivered a capture for Task 2",
    );
  });
});

describe("latestEvents", () => {
  it("keeps the newest few, newest first", () => {
    const events = [
      ev({ timestamp: "2026-10-29T10:00:00Z", task_ordinal: 0 }),
      ev({ timestamp: "2026-10-29T10:02:00Z", task_ordinal: 2 }),
      ev({ timestamp: "2026-10-29T10:01:00Z", task_ordinal: 1 }),
    ];
    expect(latestEvents(events, 2).map((e) => e.task_ordinal)).toEqual([2, 1]);
  });
});
