import { describe, it, expect } from "vitest";
import { repoLabel, repoWebUrl } from "./repo";

describe("repoLabel", () => {
  it("says one repository one way, however it is spelled", () => {
    for (const url of [
      "https://github.com/org/app.git",
      "https://github.com/org/app/",
      "ssh://git@github.com/org/app.git",
      "ssh://git@github.com:22/org/app.git",
      "git@github.com:org/app.git",
    ]) {
      expect(repoLabel(url)).toBe("github.com/org/app");
    }
    expect(repoLabel("  ")).toBeNull();
    expect(repoLabel(null)).toBeNull();
  });
});

describe("repoWebUrl", () => {
  it("links an https repository and nothing else", () => {
    expect(repoWebUrl("https://github.com/org/app.git")).toBe("https://github.com/org/app");
    expect(repoWebUrl("git@github.com:org/app.git")).toBeNull();
    expect(repoWebUrl("ssh://git@github.com/org/app")).toBeNull();
    expect(repoWebUrl("javascript:alert(1)")).toBeNull();
    expect(repoWebUrl("")).toBeNull();
  });
});
