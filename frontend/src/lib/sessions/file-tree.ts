/** A directory or file in the player repo's HEAD tree. */
export type FileTreeNode =
  | { kind: "dir"; name: string; path: string; children: FileTreeNode[] }
  | { kind: "file"; name: string; path: string; size: number };

/**
 * Fold the flat `ls-tree -r` listing into nested directories, each level
 * sorted directories first, then by name — the order every file browser
 * uses, so the eye finds `src/` before `src.rs`.
 */
export function buildFileTree(files: { path: string; size: number }[]): FileTreeNode[] {
  const root: FileTreeNode[] = [];
  const dirs = new Map<string, FileTreeNode[]>([["", root]]);
  const dirChildren = (path: string): FileTreeNode[] => {
    const known = dirs.get(path);
    if (known) return known;
    const cut = path.lastIndexOf("/");
    const parent = dirChildren(cut < 0 ? "" : path.slice(0, cut));
    const children: FileTreeNode[] = [];
    parent.push({ kind: "dir", name: path.slice(cut + 1), path, children });
    dirs.set(path, children);
    return children;
  };
  for (const f of files) {
    const cut = f.path.lastIndexOf("/");
    dirChildren(cut < 0 ? "" : f.path.slice(0, cut)).push({
      kind: "file",
      name: f.path.slice(cut + 1),
      path: f.path,
      size: f.size,
    });
  }
  for (const children of dirs.values()) {
    children.sort((a, b) =>
      a.kind === b.kind ? a.name.localeCompare(b.name) : a.kind === "dir" ? -1 : 1,
    );
  }
  return root;
}

/** Every directory on the way to `path` (`a/b/c.rs` → `a`, `a/b`). */
export function ancestorDirs(path: string): string[] {
  const parts = path.split("/").slice(0, -1);
  return parts.map((_, i) => parts.slice(0, i + 1).join("/"));
}

/**
 * The file to open first: a root README (what a visitor to any repo host
 * sees), else the first file in tree order.
 */
export function defaultFile(files: { path: string }[]): string | null {
  const readme = files.find((f) => /^readme(\.[a-z0-9]+)?$/i.test(f.path));
  if (readme) return readme.path;
  const first = (nodes: FileTreeNode[]): string | null => {
    for (const n of nodes) {
      const hit = n.kind === "file" ? n.path : first(n.children);
      if (hit) return hit;
    }
    return null;
  };
  return first(buildFileTree(files.map((f) => ({ path: f.path, size: 0 }))));
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}
