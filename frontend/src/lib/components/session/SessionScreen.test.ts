import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import SessionScreen from "./SessionScreen.svelte";
import type { SessionLinks } from "$lib/api";

vi.mock("$app/environment", () => ({ browser: true }));

// jsdom has no canvas: a no-op 2D context lets the chart draw into nothing.
const noopContext = new Proxy(
  {},
  {
    get: (_t, key) => {
      if (key === "measureText") return () => ({ width: 0 });
      if (key === "canvas") return { width: 0, height: 0 };
      if (key === "getImageData") return () => ({ data: new Uint8ClampedArray(4) });
      return () => undefined;
    },
    set: () => true,
  },
);
beforeAll(() => {
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(
    () => noopContext as unknown as CanvasRenderingContext2D,
  );
});
afterAll(() => {
  vi.restoreAllMocks();
});

const base = {
  projectName: "Weather Widget",
  joinCode: "ABC123",
  timerDigits: ["00", "12", "34"] as [string, string, string],
  participants: [],
  entries: [],
  leaderboard: [],
  reportMembers: [],
  scoreHistory: [],
  events: [],
  exitHref: "/s/ABC123",
};

function linked(all_finished: boolean): SessionLinks {
  const mk = (id: string, join_code: string, name: string, current: boolean) => ({
    id,
    join_code,
    name,
    status: all_finished ? "finished" : "running",
    project_id: "p",
    project_name: "Weather Widget",
    project_slug: null,
    created_at: "2026-10-31T10:00:00Z",
    started_at: null,
    finished_at: null,
    player_count: 2,
    current,
  });
  return {
    group_id: "g",
    all_finished,
    sessions: [mk("s1", "ABC123", "Round 1", true), mk("s2", "XYZ789", "Round 2", false)],
    leaderboard: [
      {
        user_id: "u1",
        display_name: "Ada",
        total_points: 70,
        sessions_played: 2,
        per_session: [
          { session_id: "s1", points: 30, rank: 1 },
          { session_id: "s2", points: 40, rank: 1 },
        ],
      },
    ],
  };
}

describe("SessionScreen linked sessions", () => {
  afterEach(() => cleanup());

  it("has no slider for a session linked to nothing", () => {
    render(SessionScreen, { props: { ...base, phase: "active" } });
    expect(screen.queryByTestId("screen-slider")).toBeNull();
  });

  it("shows a stop per linked session and Overall, this one current, and moves on a click or an arrow key", async () => {
    const onSlide = vi.fn();
    render(SessionScreen, { props: { ...base, phase: "active", links: linked(false), onSlide } });
    const stops = screen.getAllByTestId("screen-slider-stop");
    expect(stops.map((s) => s.textContent)).toEqual(["Round 1", "Round 2", "Overall"]);
    expect(stops[0].getAttribute("aria-current")).toBe("true");

    await fireEvent.click(stops[1]);
    expect(onSlide).toHaveBeenLastCalledWith({ code: "XYZ789" });
    await fireEvent.click(stops[0]);
    expect(onSlide).toHaveBeenCalledTimes(1);

    await fireEvent.click(screen.getByTestId("screen-slider-next"));
    expect(onSlide).toHaveBeenLastCalledWith({ code: "XYZ789" });
    // Left from the first stop wraps to Overall.
    await fireEvent.keyDown(window, { key: "ArrowLeft" });
    expect(onSlide).toHaveBeenCalledTimes(3);
    expect(onSlide).toHaveBeenLastCalledWith("overall");
  });

  it("shows the combined standings live while the rooms play, and as final once every one has ended", async () => {
    const { unmount: unmountLive } = render(SessionScreen, {
      props: { ...base, phase: "active", links: linked(false), slide: "overall" },
    });
    expect(screen.getByTestId("screen-status").textContent).toBe("Overall · live");
    expect(screen.getByTestId("screen-overall-live")).toBeTruthy();
    expect(screen.getByTestId("screen-combined-total").textContent).toBe("70");
    unmountLive();

    const onSlide = vi.fn();
    const { unmount } = render(SessionScreen, {
      props: { ...base, phase: "finished", links: linked(true), onSlide },
    });
    const stops = screen.getAllByTestId("screen-slider-stop");
    expect(stops.map((s) => s.getAttribute("data-key"))).toEqual(["ABC123", "XYZ789", "overall"]);
    expect(screen.queryByTestId("screen-overall")).toBeNull();
    await fireEvent.click(stops[2]);
    expect(onSlide).toHaveBeenLastCalledWith("overall");
    unmount();

    render(SessionScreen, {
      props: { ...base, phase: "finished", links: linked(true), slide: "overall", onSlide },
    });
    expect(screen.getByTestId("screen-status").textContent).toBe("Final standings");
    expect(screen.queryByTestId("screen-overall-live")).toBeNull();
    expect(screen.getByTestId("screen-overall").textContent).toContain(
      "Final standings · 2 sessions",
    );
    expect(screen.getByTestId("screen-combined-total").textContent).toBe("70");
    expect(screen.getByTestId("session-screen").getAttribute("data-slide")).toBe("overall");
    const current = screen
      .getAllByTestId("screen-slider-stop")
      .find((s) => s.getAttribute("aria-current") === "true");
    expect(current?.getAttribute("data-key")).toBe("overall");
    // Left from Overall goes back to the last session.
    await fireEvent.keyDown(window, { key: "ArrowLeft" });
    expect(onSlide).toHaveBeenLastCalledWith({ code: "XYZ789" });
  });
});
