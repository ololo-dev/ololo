import { describe, expect, it } from "vitest";
import { ikAvatar, ikCover, ikWidth } from "./imagekit";

describe("imagekit helpers", () => {
  it("appends a 2x square transform with auto format for ImageKit avatars", () => {
    expect(ikAvatar("https://ik.imagekit.io/abc/av.png", 32)).toBe(
      "https://ik.imagekit.io/abc/av.png?tr=w-64,h-64,f-auto",
    );
  });

  it("leaves externally hosted avatars untouched", () => {
    const external = "https://example.com/avatars/u/123.png?v=4";
    expect(ikAvatar(external, 32)).toBe(external);
  });

  it("leaves blob/preview URLs untouched", () => {
    const blob = "blob:http://localhost:5173/1234";
    expect(ikCover(blob, 1200, 320)).toBe(blob);
  });

  it("returns empty string for missing URLs", () => {
    expect(ikAvatar(null, 32)).toBe("");
    expect(ikAvatar(undefined, 32)).toBe("");
  });

  it("crops covers to an exact box with auto focus and format", () => {
    expect(ikCover("https://ik.imagekit.io/abc/cover.jpg", 760, 380)).toBe(
      "https://ik.imagekit.io/abc/cover.jpg?tr=w-760,h-380,fo-auto,f-auto",
    );
  });

  it("replaces an existing tr parameter instead of stacking", () => {
    expect(ikWidth("https://ik.imagekit.io/abc/bg.jpg?tr=w-100", 1600)).toBe(
      "https://ik.imagekit.io/abc/bg.jpg?tr=w-1600,f-auto",
    );
  });
});
