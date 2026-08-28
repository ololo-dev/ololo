import { describe, expect, it } from "vitest";
import { IMAGE_ACCEPT_ATTR, MAX_IMAGE_BYTES, validateImageFile } from "./image-upload";

function file(name: string, type: string, size = 1024): File {
  const f = new File([new Uint8Array(1)], name, { type });
  // File size is read-only; override for the size check.
  Object.defineProperty(f, "size", { value: size });
  return f;
}

describe("validateImageFile", () => {
  it("accepts every browser-renderable format we allow", () => {
    for (const [name, type] of [
      ["a.jpg", "image/jpeg"],
      ["a.png", "image/png"],
      ["a.webp", "image/webp"],
      ["a.gif", "image/gif"],
      ["a.avif", "image/avif"],
    ]) {
      expect(validateImageFile(file(name, type)), `${type} should pass`).toBeUndefined();
    }
  });

  it("accepts a file whose MIME type the OS could not resolve, by extension", () => {
    // Some OS/browser combinations report an empty type for valid images;
    // rejecting those made real JPEGs un-uploadable.
    expect(validateImageFile(file("holiday.JPG", ""))).toBeUndefined();
    expect(validateImageFile(file("shot.webp", ""))).toBeUndefined();
  });

  it("rejects formats browsers cannot display or that carry scripts", () => {
    // SVG can carry scripts; HEIC renders only in Safari.
    expect(validateImageFile(file("x.svg", "image/svg+xml"))).toMatch(/unsupported/i);
    expect(validateImageFile(file("x.heic", "image/heic"))).toMatch(/unsupported/i);
    expect(validateImageFile(file("x.pdf", "application/pdf"))).toMatch(/unsupported/i);
    // Unknown type *and* unknown extension.
    expect(validateImageFile(file("notes.txt", ""))).toMatch(/unsupported/i);
  });

  it("rejects files over the size cap", () => {
    expect(validateImageFile(file("big.png", "image/png", MAX_IMAGE_BYTES + 1))).toMatch(/5 MB/);
    expect(validateImageFile(file("ok.png", "image/png", MAX_IMAGE_BYTES))).toBeUndefined();
  });

  it("offers both MIME types and extensions to the file picker", () => {
    // Extensions matter: a picker filtering purely on MIME greys out files
    // whose type the OS cannot resolve.
    expect(IMAGE_ACCEPT_ATTR).toContain("image/webp");
    expect(IMAGE_ACCEPT_ATTR).toContain(".webp");
    expect(IMAGE_ACCEPT_ATTR).not.toContain("svg");
  });
});
