import { describe, it, expect } from "vitest";
import { fileName, galleryEntries } from "./artifacts";
import type { PlayerArtifactRef } from "$lib/types/arena";

const url = (probeId: string, i: number) => (i > 0 ? `/a/${probeId}?i=${i}` : `/a/${probeId}`);

describe("galleryEntries", () => {
  it("names every delivered file by itself", () => {
    const ref = {
      probe_id: "pr1",
      content_type: "image/png",
      label: ".ololo/artifacts/8dd/receipt-desktop.png",
      file_count: 2,
      files: [
        ".ololo/artifacts/8dd/receipt-desktop.png",
        ".ololo/artifacts/8dd/receipt-mobile.png",
      ],
    } as PlayerArtifactRef;
    expect(galleryEntries(ref, url)).toEqual([
      { key: "pr1:0", src: "/a/pr1", content_type: "image/png", label: "receipt-desktop.png" },
      { key: "pr1:1", src: "/a/pr1?i=1", content_type: "image/png", label: "receipt-mobile.png" },
    ]);
  });

  it("falls back to a position when the ref carries no file list", () => {
    const ref = {
      probe_id: "pr1",
      content_type: "image/png",
      label: ".ololo/artifacts/8dd/book-desktop.png",
      file_count: 2,
    } as PlayerArtifactRef;
    expect(galleryEntries(ref, url).map((e) => e.label)).toEqual([
      "book-desktop.png (1/2)",
      "book-desktop.png (2/2)",
    ]);
  });

  it("keys a single-file delivery by the probe alone", () => {
    const ref = {
      probe_id: "pr1",
      content_type: "video/webm",
      label: ".ololo/artifacts/8dd/run.webm",
    } as PlayerArtifactRef;
    expect(galleryEntries(ref, url)).toEqual([
      { key: "pr1", src: "/a/pr1", content_type: "video/webm", label: "run.webm" },
    ]);
  });

  it("takes the last segment as the name", () => {
    expect(fileName(".ololo/artifacts/8dd/shot.png")).toBe("shot.png");
    expect(fileName("shot.png")).toBe("shot.png");
  });
});
