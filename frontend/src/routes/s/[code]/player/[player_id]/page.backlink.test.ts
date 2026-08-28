import { describe, it, expect } from "vitest";
import { makeSnapshot, renderPage } from "./page.test-helpers";

/**
 * The way out of a player's page used to be an arrow followed by the bare
 * join code — "← TEST01". It named neither the destination nor the fact that
 * it was a way back, and the code means nothing to anyone who did not type it
 * into the CLI themselves.
 */
describe("player page back link", () => {
  function backLink() {
    return document.querySelector('a[href="/s/TEST01"]');
  }

  it("Says it goes back, and to what", () => {
    renderPage({
      live: true,
      snapshot: makeSnapshot({ session_status: "running" }),
      token: null,
      playerName: "Test Player",
    });
    const text = backLink()?.textContent?.replace(/\s+/g, " ").trim();
    expect(text).toContain("Back to");
    // While the session runs, /s/<code> is the live board.
    expect(text).toContain("live board");
  });

  it("Names the report once the session has ended", () => {
    renderPage({
      live: false,
      snapshot: makeSnapshot({ session_status: "finished" }),
      token: null,
      playerName: "Test Player",
    });
    const text = backLink()?.textContent?.replace(/\s+/g, " ").trim();
    expect(text).toContain("Back to session results");
    expect(text).not.toContain("live board");
  });

  it("Keeps the code visible, since it is still the session's identifier", () => {
    renderPage({
      live: false,
      snapshot: makeSnapshot({ session_status: "finished" }),
      token: null,
      playerName: "Test Player",
    });
    expect(backLink()?.textContent).toContain("TEST01");
  });
});
