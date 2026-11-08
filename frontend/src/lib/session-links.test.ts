import { describe, expect, it } from "vitest";
import type { SessionLinks } from "$lib/api";
import { sessionLabel, slideFromUrl, slideStops, stopBy } from "./session-links";

function group(all_finished: boolean, current = "AAA111"): SessionLinks {
  const mk = (join_code: string, name: string, project_name: string | null) => ({
    id: `id-${join_code}`,
    join_code,
    name,
    status: all_finished ? "finished" : "running",
    project_id: "p",
    project_name,
    project_slug: null,
    created_at: "2026-10-31T10:00:00Z",
    started_at: null,
    finished_at: null,
    player_count: 3,
    current: join_code === current,
  });
  return {
    group_id: "g",
    all_finished,
    leaderboard: [],
    sessions: [
      mk("AAA111", "Round 1", "FizzBuzz"),
      mk("BBB222", "", "Weather Widget"),
      mk("CCC333", "", null),
    ],
  };
}

describe("slideStops", () => {
  it("has no stops without a group, or for a group of one", () => {
    expect(slideStops(null, "session")).toEqual([]);
    const one = group(false);
    one.sessions = one.sessions.slice(0, 1);
    expect(slideStops(one, "session")).toEqual([]);
  });

  it("names a stop per session, the current one marked, an unnamed room by its join code", () => {
    const stops = slideStops(group(false), "session");
    expect(stops.map((s) => s.label)).toEqual(["Round 1", "BBB222", "CCC333", "Overall"]);
    expect(stops.map((s) => s.current)).toEqual([true, false, false, false]);
    expect(stops[1].target).toEqual({ code: "BBB222" });
    // The rooms run side by side: the Overall stop is there from the start.
    expect(stops[3]).toEqual({
      key: "overall",
      label: "Overall",
      target: "overall",
      current: false,
    });
  });

  it("marks the Overall stop current on the overall slide", () => {
    const onSession = slideStops(group(true), "session");
    expect(onSession.map((s) => s.key)).toEqual(["AAA111", "BBB222", "CCC333", "overall"]);
    expect(onSession.map((s) => s.current)).toEqual([true, false, false, false]);
    const onOverall = slideStops(group(true), "overall");
    expect(onOverall.map((s) => s.current)).toEqual([false, false, false, true]);
    expect(onOverall[3].target).toBe("overall");
  });
});

describe("stopBy", () => {
  it("walks forward and back, wrapping round", () => {
    const stops = slideStops(group(true), "session");
    expect(stopBy(stops, 1)?.key).toBe("BBB222");
    expect(stopBy(stops, -1)?.key).toBe("overall");
    expect(stopBy(slideStops(group(false), "session"), -1)?.key).toBe("overall");
    const fromLast = slideStops(group(true), "overall");
    expect(stopBy(fromLast, 1)?.key).toBe("AAA111");
  });

  it("goes nowhere without stops", () => {
    expect(stopBy([], 1)).toBeNull();
  });
});

describe("slideFromUrl", () => {
  it("reads ?slide=overall and nothing else", () => {
    expect(slideFromUrl(new URL("http://x/s/AAA111?view=screen&slide=overall"))).toBe("overall");
    expect(slideFromUrl(new URL("http://x/s/AAA111?view=screen"))).toBe("session");
    expect(slideFromUrl(new URL("http://x/s/AAA111?slide=nope"))).toBe("session");
  });
});

describe("sessionLabel", () => {
  it("tells twins apart by their join code, and leaves a lone name alone", () => {
    const g = group(false);
    g.sessions[1] = { ...g.sessions[1], name: "Round 1" };
    expect(sessionLabel(g.sessions, 0)).toBe("Round 1 · AAA111");
    expect(sessionLabel(g.sessions, 1)).toBe("Round 1 · BBB222");
    expect(sessionLabel(g.sessions, 2)).toBe("CCC333");
    expect(slideStops(g, "session").map((s) => s.label)).toEqual([
      "Round 1 · AAA111",
      "Round 1 · BBB222",
      "CCC333",
      "Overall",
    ]);
  });

  it("never names a room after the project: the group runs one project", () => {
    const g = group(false);
    g.sessions = g.sessions.map((s) => ({ ...s, name: "", project_name: "FizzBuzz" }));
    expect(g.sessions.map((_, i) => sessionLabel(g.sessions, i))).toEqual([
      "AAA111",
      "BBB222",
      "CCC333",
    ]);
    // A room created from the web is named after its project: no name of its own.
    g.sessions = g.sessions.map((s) => ({ ...s, name: "FizzBuzz" }));
    g.sessions[2] = { ...g.sessions[2], name: "Round 3" };
    expect(g.sessions.map((_, i) => sessionLabel(g.sessions, i))).toEqual([
      "AAA111",
      "BBB222",
      "Round 3",
    ]);
  });
});
