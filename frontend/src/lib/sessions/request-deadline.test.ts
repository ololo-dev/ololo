import { describe, expect, it } from "vitest";
import { formatCountdown, requestOpenUntil } from "./request-deadline";

describe("requestOpenUntil", () => {
  it("adds the stamped seconds to the dispatch time", () => {
    const cmd =
      "# ARTIFACT REQUEST from ux: shot\n# Open for 287s more.\n# Save under x/\ntest -f x/a";
    expect(requestOpenUntil(cmd, "2026-09-23T10:00:00Z")).toBe(Date.parse("2026-09-23T10:04:47Z"));
  });

  it("is null without a stamp or a dispatch time", () => {
    expect(
      requestOpenUntil("# ARTIFACT REQUEST from ux: shot\ntest -f x", "2026-09-23T10:00:00Z"),
    ).toBeNull();
    expect(requestOpenUntil("# Open for 5s more.", null)).toBeNull();
  });
});

describe("formatCountdown", () => {
  it("prints minutes and zero-padded seconds, never negative", () => {
    expect(formatCountdown(247_900)).toBe("4:07");
    expect(formatCountdown(59_000)).toBe("0:59");
    expect(formatCountdown(-5_000)).toBe("0:00");
  });
});
