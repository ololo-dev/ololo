import { describe, expect, it } from "vitest";
import { ancestorDirs, buildFileTree, defaultFile, formatBytes } from "./file-tree";

describe("buildFileTree", () => {
  it("nests files under their directories, directories first", () => {
    const tree = buildFileTree([
      { path: "src/main.rs", size: 10 },
      { path: "Cargo.toml", size: 3 },
      { path: "src/lib/mod.rs", size: 7 },
      { path: "README.md", size: 5 },
    ]);
    expect(tree.map((n) => n.name)).toEqual(["src", "Cargo.toml", "README.md"]);
    const src = tree[0];
    expect(src.kind).toBe("dir");
    if (src.kind !== "dir") return;
    expect(src.children.map((n) => `${n.kind}:${n.path}`)).toEqual([
      "dir:src/lib",
      "file:src/main.rs",
    ]);
  });
});

describe("ancestorDirs", () => {
  it("lists every directory on the way to a file", () => {
    expect(ancestorDirs("a/b/c.rs")).toEqual(["a", "a/b"]);
    expect(ancestorDirs("top.rs")).toEqual([]);
  });
});

describe("defaultFile", () => {
  it("opens the root README first", () => {
    expect(
      defaultFile([{ path: "a/README.md" }, { path: "src/x.rs" }, { path: "Readme.MD" }]),
    ).toBe("Readme.MD");
  });

  it("falls back to the first file in tree order", () => {
    expect(defaultFile([{ path: "z.rs" }, { path: "src/a.rs" }])).toBe("src/a.rs");
    expect(defaultFile([])).toBeNull();
  });
});

describe("formatBytes", () => {
  it("scales the unit", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(2048)).toBe("2.0 KB");
    expect(formatBytes(3 * 1024 * 1024)).toBe("3.0 MB");
  });
});
