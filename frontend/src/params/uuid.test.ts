import { describe, it, expect } from "vitest";
import { match } from "./uuid";

describe("uuid param matcher", () => {
  it("returns true for a valid UUID v4", () => {
    expect(match("550e8400-e29b-41d4-a716-446655440000")).toBe(true);
  });

  it("returns false for a slug string", () => {
    expect(match("my-project-slug")).toBe(false);
  });

  it("returns false for an integer string", () => {
    expect(match("123")).toBe(false);
  });

  it("returns false for an empty string", () => {
    expect(match("")).toBe(false);
  });

  it("returns false for a non-uuid string", () => {
    expect(match("not-a-uuid-at-all")).toBe(false);
  });

  it("returns false for a UUID with an invalid character", () => {
    expect(match("550e8400-e29b-41d4-a716-44665544000g")).toBe(false);
  });
});
