import { describe, it, expect } from "vitest";
import { sessionLinkLabel, statusLabels } from "./session-status";

describe("sessionLinkLabel", () => {
  it("Names the finished report, which is what most links point at", () => {
    expect(sessionLinkLabel("finished")).toBe("Session results");
    // A cancelled session still has a report page, so it reads the same.
    expect(sessionLinkLabel("cancelled")).toBe("Session results");
  });

  it("Names the lobby, since /s/<code> redirects there before a start", () => {
    expect(sessionLinkLabel("lobby")).toBe("Session lobby");
  });

  it("Names the live board while the session is in play", () => {
    expect(sessionLinkLabel("running")).toBe("Live board");
    expect(sessionLinkLabel("paused")).toBe("Live board");
  });

  it("Still says something for a status it has never heard of", () => {
    // A status added server-side must degrade to a usable label rather than
    // leaving a link with no text at all.
    expect(sessionLinkLabel("something-new")).toBe("Live board");
  });
});

describe("statusLabels", () => {
  it("Covers every session status, so none renders as a raw enum name", () => {
    for (const status of ["lobby", "running", "paused", "finished", "cancelled"]) {
      expect(statusLabels[status], status).toBeTruthy();
      expect(statusLabels[status]).not.toBe(status);
    }
  });
});
