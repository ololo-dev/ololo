import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/svelte";
import ScreenCombinedLeaderboard from "./ScreenCombinedLeaderboard.svelte";
import type { SessionLinks } from "$lib/api";

function session(id: string, name: string, current = false) {
  return {
    id,
    join_code: id.toUpperCase(),
    name,
    status: "finished",
    project_id: "p",
    project_name: "FizzBuzz",
    project_slug: null,
    created_at: "2026-10-31T10:00:00Z",
    started_at: null,
    finished_at: null,
    player_count: 2,
    current,
  };
}

/** `rowCount` players spread over `sessionCount` rooms, one room each. */
function links(sessionCount: number, rowCount: number): SessionLinks {
  const sessions = Array.from({ length: sessionCount }, (_, i) =>
    session(`s${i + 1}`, `Room ${i + 1}`, i === 0),
  );
  const leaderboard = Array.from({ length: rowCount }, (_, i) => ({
    user_id: `u${i}`,
    display_name: `Player ${i}`,
    total_points: 100 - i,
    sessions_played: 1,
    per_session: [{ session_id: `s${(i % sessionCount) + 1}`, points: 100 - i, rank: 1 }],
  }));
  return { group_id: "g", all_finished: true, sessions, leaderboard };
}

describe("ScreenCombinedLeaderboard", () => {
  afterEach(() => cleanup());

  it("lists every room's players by points, each with the room they played", () => {
    render(ScreenCombinedLeaderboard, { props: { links: links(2, 3) } });
    const rows = screen.getAllByTestId("screen-combined-row");
    expect(rows).toHaveLength(3);
    expect(rows[0].textContent).toContain("Player 0");
    const origins = screen.getAllByTestId("screen-combined-origin").map((e) => e.textContent);
    expect(origins).toEqual(["Room 1", "Room 2", "Room 1"]);
    expect(screen.getAllByTestId("screen-combined-total").map((e) => e.textContent)).toEqual([
      "100",
      "99",
      "98",
    ]);
    expect(screen.getByTestId("screen-combined").getAttribute("data-columns")).toBe("1");
  });

  it("spells out the rooms and points of a player who sat in several", () => {
    const g = links(2, 1);
    g.leaderboard[0].sessions_played = 2;
    g.leaderboard[0].total_points = 70;
    g.leaderboard[0].per_session = [
      { session_id: "s1", points: 30, rank: 2 },
      { session_id: "s2", points: 40, rank: 1 },
    ];
    render(ScreenCombinedLeaderboard, { props: { links: g } });
    expect(screen.getByTestId("screen-combined-origin").textContent).toBe("Room 1 30 · Room 2 40");
    expect(screen.getByTestId("screen-combined-total").textContent).toBe("70");
  });

  it("runs in two columns past twelve players", () => {
    render(ScreenCombinedLeaderboard, { props: { links: links(3, 20) } });
    expect(screen.getByTestId("screen-combined").getAttribute("data-columns")).toBe("2");
    expect(screen.getAllByTestId("screen-combined-row")).toHaveLength(20);
  });

  it("says so when nobody scored", () => {
    render(ScreenCombinedLeaderboard, {
      props: { links: links(2, 0), emptyMessage: "Nothing yet." },
    });
    expect(screen.getByTestId("screen-combined").textContent).toContain("Nothing yet.");
  });
});
