import { describe, it, expect, afterEach, beforeEach } from "vitest";
import { screen, fireEvent } from "@testing-library/svelte";
import { makeSnapshot, renderPage } from "./page.test-helpers";
import type { PlayerSessionReport } from "$lib/types/arena";

/**
 * A finished session opens on the report: it answers "how did I do" in prose,
 * and it is the one view that summarises what every other view shows in
 * detail. A live session still opens on the chat.
 */

const report: PlayerSessionReport = {
  judge_name: "The Debrief",
  judge_slug: "general",
  markdown: "## What you built\n\nA server that survives a bad statement.",
  created_at: "2026-08-20T18:00:00Z",
};

// The view choice is persisted, and this jsdom's localStorage outlives the
// file — a stored choice here would decide the default view for every other
// player-page test that runs after it.
beforeEach(() => {
  localStorage.clear();
});

afterEach(() => {
  localStorage.clear();
});

describe("routes/s/[code]/player/[player_id] — session report", () => {
  it("opens a finished session on the report", () => {
    renderPage({
      live: false,
      snapshot: makeSnapshot({ session_report: report }),
      token: null,
      playerName: "Test Player",
    });
    expect(screen.getByTestId("session-report")).not.toBeNull();
    expect(screen.getByText(/survives a bad statement/)).not.toBeNull();
  });

  it("opens a finished session on the report while it is still being written", () => {
    renderPage({
      live: false,
      judgesSettling: true,
      snapshot: makeSnapshot(),
      token: null,
      playerName: "Test Player",
    });
    expect(screen.getByTestId("session-report-pending")).not.toBeNull();
  });

  it("still opens a live session on the chat", () => {
    renderPage({
      live: true,
      snapshot: makeSnapshot(),
      token: "t",
      playerName: "Test Player",
    });
    expect(screen.queryByTestId("session-report")).toBeNull();
    // And the report is not offered as a view while the session runs.
    expect(screen.queryByRole("button", { name: "Report" })).toBeNull();
  });

  it("falls back to the full record when a finished session has no report", () => {
    renderPage({
      live: false,
      snapshot: makeSnapshot(),
      token: null,
      playerName: "Test Player",
    });
    expect(screen.queryByTestId("session-report")).toBeNull();
  });

  it("keeps the player's own choice for this session", async () => {
    renderPage({
      live: false,
      snapshot: makeSnapshot({ session_report: report }),
      token: null,
      playerName: "Test Player",
    });
    await fireEvent.click(screen.getByRole("button", { name: "Chat" }));
    expect(screen.queryByTestId("session-report")).toBeNull();
    // The choice is scoped to this session, not stored as a preference for life.
    expect(localStorage.getItem("player:task-view:player-1")).toBe("chat");
    expect(localStorage.getItem("player:task-view")).toBeNull();
  });
});
