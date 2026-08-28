import { describe, it, expect, beforeEach } from "vitest";
import { screen, fireEvent } from "@testing-library/svelte";
import { tick } from "svelte";
import { makeSnapshot, makeTaskSummary, makeProbe, renderPage } from "./page.test-helpers";

/**
 * Which view the page opens with, and how the choice sticks.
 *
 * A player mid-race follows the conversation; someone reviewing a finished
 * session wants the whole record. So the default follows the session's
 * state — until the player says otherwise.
 */
describe("player page view", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  const snapshot = makeSnapshot({
    tasks: [
      makeTaskSummary({
        task_id: "task-1",
        ordinal: 1,
        scheduler_state: { state: "active", activated_at: null, deadline_at: null },
      }),
    ],
    probes: [makeProbe({ resolved_at: "2026-08-02T10:00:00Z", state: "resolved" })],
  });

  it("Opens a running session on the chat", () => {
    renderPage({ live: true, snapshot, token: "t", playerName: "p" });
    expect(screen.getByTestId("task-chat")).not.toBeNull();
    expect(screen.getByTestId("task-view-chat").getAttribute("aria-pressed")).toBe("true");
  });

  it("Replaces the whole page, not just one tab", () => {
    renderPage({ live: true, snapshot, token: "t", playerName: "p" });
    // No section tabs in the chat view: Changes, Judges and Stats are
    // what a player under a clock does not need.
    expect(screen.queryByTestId("ptab-tasks")).toBeNull();
    expect(screen.queryByTestId("ptab-judges")).toBeNull();
    // The header stays — score, rank and the clock are the point.
    expect(screen.getByText("Score")).not.toBeNull();
  });

  it("Brings the sections back in the detailed view", async () => {
    renderPage({ live: true, snapshot, token: "t", playerName: "p" });
    await fireEvent.click(screen.getByTestId("task-view-details"));
    await tick();
    expect(screen.getByTestId("ptab-tasks")).not.toBeNull();
    expect(screen.getByTestId("ptab-judges")).not.toBeNull();
  });

  it("Opens a finished session on the full record", () => {
    renderPage({ live: false, snapshot, token: null, playerName: "p" });
    expect(screen.queryByTestId("task-chat")).toBeNull();
    expect(screen.getByTestId("task-view-details").getAttribute("aria-pressed")).toBe("true");
  });

  it("Offers no slides view any more", () => {
    renderPage({ live: true, snapshot, token: "t", playerName: "p" });
    expect(screen.queryByTestId("task-view-slides")).toBeNull();
  });

  it("A stale stored 'slides' choice falls back to the default", () => {
    localStorage.setItem("player:task-view:player-1", "slides");
    renderPage({ live: true, snapshot, token: "t", playerName: "p" });
    expect(screen.getByTestId("task-chat")).not.toBeNull();
    expect(screen.getByTestId("task-view-chat").getAttribute("aria-pressed")).toBe("true");
  });

  it("Remembers the choice, so a reload does not undo it", async () => {
    const first = renderPage({ live: true, snapshot, token: "t", playerName: "p" });
    await fireEvent.click(screen.getByTestId("task-view-details"));
    await tick();
    first.unmount();

    // Same running session, rendered again: the explicit choice wins over
    // the live-session default.
    renderPage({ live: true, snapshot, token: "t", playerName: "p" });
    expect(screen.queryByTestId("task-chat")).toBeNull();
    expect(screen.getByTestId("task-view-details").getAttribute("aria-pressed")).toBe("true");
  });
});
