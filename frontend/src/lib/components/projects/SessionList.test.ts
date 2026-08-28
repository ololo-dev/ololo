import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/svelte";
import SessionList from "./SessionList.svelte";
import type { Session } from "$lib/api";

function session(over: Partial<Session> = {}): Session {
  return {
    id: "s1",
    join_code: "F3W7AC",
    name: "Flower Watering Reminder",
    status: "finished",
    created_at: "2026-08-02T09:14:00Z",
    player_count: 1,
    best_player_name: "Anod",
    best_score: -55,
    ...over,
  } as Session;
}

function props(sessions: Session[]) {
  return {
    sessions,
    sessionGroups: [{ label: "Completed", status: "finished", sessions }],
    isAdmin: false,
    showCancelled: false,
    wsClient: null,
  };
}

describe("SessionList", () => {
  it("Dates a finished session down to the minute", () => {
    render(SessionList, props([session()]));
    // A busy project runs several sessions a day; the date alone cannot tell
    // two of them apart.
    expect(screen.getByText("Aug 2, 2026")).not.toBeNull();
    expect(screen.getByText("09:14 UTC")).not.toBeNull();
  });

  it("States the zone, so a reader is not left guessing whose clock it is", () => {
    render(SessionList, props([session({ id: "s2", created_at: "2026-08-02T23:50:00Z" })]));
    // Rendered on the server as well as in the browser: a local time would
    // disagree with itself across hydration.
    expect(screen.getByText("23:50 UTC")).not.toBeNull();
  });

  it("Tells two same-day sessions apart", () => {
    render(
      SessionList,
      props([
        session({ id: "s1", join_code: "AAA111", created_at: "2026-08-02T09:14:00Z" }),
        session({ id: "s2", join_code: "BBB222", created_at: "2026-08-02T17:02:00Z" }),
      ]),
    );
    expect(screen.getAllByText("Aug 2, 2026").length).toBe(2);
    expect(screen.getByText("09:14 UTC")).not.toBeNull();
    expect(screen.getByText("17:02 UTC")).not.toBeNull();
  });
});
