import { describe, expect, it } from "vitest";
import { displayTrailers, parseCommitMessage } from "./commit-message";

describe("commit-message", () => {
  it("a format-0 message is a bare subject", () => {
    const p = parseCommitMessage("feat(abc): Build the widget");
    expect(p.subject).toBe("feat(abc): Build the widget");
    expect(p.body).toBe("");
    expect(p.trailers).toEqual([]);
  });

  it("a format-1 message splits into subject and Ololo trailers", () => {
    const msg = [
      "probe(abc): #7 Build the widget",
      "",
      "Ololo-Format: 1",
      "Ololo-Task: abc",
      "Ololo-Task-Title: Build the widget",
      "Ololo-Probe-Seq: 7",
      "Ololo-Timestamp: 2026-09-22T10:00:00Z",
    ].join("\n");
    const p = parseCommitMessage(msg);
    expect(p.subject).toBe("probe(abc): #7 Build the widget");
    expect(p.body).toBe("");
    expect(p.trailers.map((t) => t.key)).toEqual([
      "Format",
      "Task",
      "Task-Title",
      "Probe-Seq",
      "Timestamp",
    ]);
    expect(displayTrailers(p)).toEqual([
      { key: "task title", value: "Build the widget" },
      { key: "probe", value: "#7" },
      { key: "timestamp", value: "2026-09-22T10:00:00Z" },
    ]);
  });

  it("a prose body without trailers stays a body", () => {
    const p = parseCommitMessage("Initial commit\n\nThis explains: the change\nin prose.");
    expect(p.body).toBe("This explains: the change\nin prose.");
    expect(p.trailers).toEqual([]);
  });
});
