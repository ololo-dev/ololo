import { describe, expect, it } from "vitest";
import { MAX_HIGHLIGHT_BYTES, highlightToHtml, languageFor } from "./highlight";

describe("languageFor", () => {
  it("maps extensions through shiki's aliases", async () => {
    expect(await languageFor("src/main.rs")).toBe("rs");
    expect(await languageFor("web/App.TSX")).toBe("tsx");
    expect(await languageFor("Cargo.toml")).toBe("toml");
    expect(await languageFor("include/util.h")).toBe("c");
  });

  it("knows the files named after their language", async () => {
    expect(await languageFor("Dockerfile")).toBe("dockerfile");
    expect(await languageFor("deploy/Dockerfile.prod")).toBe("dockerfile");
    expect(await languageFor("Makefile")).toBe("makefile");
  });

  it("leaves unknown and extensionless files plain", async () => {
    expect(await languageFor("notes.weird")).toBeNull();
    expect(await languageFor("LICENSE")).toBeNull();
    expect(await languageFor(".gitignore")).toBeNull();
  });
});

describe("highlightToHtml", () => {
  it("colours code and escapes it", async () => {
    const html = await highlightToHtml('fn main() { println!("<b>"); }\n', "main.rs");
    expect(html).toContain('<span style="color:');
    expect(html).toContain("&#x3C;b>");
    expect(html).not.toContain("<b>");
  });

  it("returns null for plain text and for huge files", async () => {
    expect(await highlightToHtml("hello", "notes.weird")).toBeNull();
    expect(await highlightToHtml("x".repeat(MAX_HIGHLIGHT_BYTES + 1), "big.rs")).toBeNull();
  });
});
