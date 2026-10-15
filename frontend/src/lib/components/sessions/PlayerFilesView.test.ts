import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import PlayerFilesView from "./PlayerFilesView.svelte";

const SHA = "a".repeat(40);
const BASE = "/api/sessions/ABC123/players/p-1/files";

const LISTING = {
  head: { sha: SHA, subject: "feat: add parser", author_time: "2026-10-14T10:00:00Z" },
  files: [
    { path: "README.md", size: 6 },
    { path: "src/main.rs", size: 13 },
    { path: "logo.bin", size: 4 },
  ],
  truncated: false,
};

const CONTENT: Record<string, unknown> = {
  "README.md": { path: "README.md", size: 6, binary: false, too_large: false, content: "# Demo\n" },
  "src/main.rs": {
    path: "src/main.rs",
    size: 13,
    binary: false,
    too_large: false,
    content: "fn main() {}\n",
  },
  "logo.bin": { path: "logo.bin", size: 4, binary: true, too_large: false, content: null },
};

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

describe("PlayerFilesView", () => {
  let fetchMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = new URL(String(input), "http://localhost");
      if (url.pathname === BASE) return json(LISTING);
      if (url.pathname === `${BASE}/content`) {
        const body = CONTENT[url.searchParams.get("path") ?? ""];
        return body ? json(body) : json({ error: "not_found" }, 404);
      }
      return json({}, 404);
    });
    vi.stubGlobal("fetch", fetchMock);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  const props = { sessionCode: "ABC123", playerId: "p-1", revision: SHA };

  it("shows HEAD's commit and opens the README", async () => {
    render(PlayerFilesView, { props });
    await waitFor(() =>
      expect(screen.getByTestId("files-content").textContent).toContain("# Demo"),
    );
    expect(screen.getByTestId("files-head-sha").textContent).toBe("aaaaaaa");
    expect(screen.getByText("feat: add parser")).toBeInTheDocument();
    // The content request is pinned to the listed commit.
    const contentCall = fetchMock.mock.calls
      .map(([u]) => new URL(String(u), "http://localhost"))
      .find((u) => u.pathname.endsWith("/content"));
    expect(contentCall?.searchParams.get("sha")).toBe(SHA);
  });

  it("opens a nested file from the tree", async () => {
    render(PlayerFilesView, { props });
    await fireEvent.click(await screen.findByTestId("file-src/main.rs"));
    await waitFor(() =>
      expect(screen.getByTestId("files-content").textContent).toContain("fn main() {}"),
    );
    expect(screen.getByTestId("files-selected-path").textContent).toBe("src/main.rs");
  });

  it("colours a source file once its grammar loads", async () => {
    render(PlayerFilesView, { props });
    await fireEvent.click(await screen.findByTestId("file-src/main.rs"));
    await waitFor(
      () => {
        const pre = screen.getByTestId("files-content").querySelector("[data-highlighted]");
        expect(pre?.querySelector('span[style*="color"]')).not.toBeNull();
      },
      { timeout: 5000 },
    );
    expect(screen.getByTestId("files-content").textContent).toContain("fn main() {}");
  });

  it("names a binary file instead of dumping it", async () => {
    render(PlayerFilesView, { props });
    await fireEvent.click(await screen.findByTestId("file-logo.bin"));
    expect(await screen.findByText(/Binary file \(4 B\)/)).toBeInTheDocument();
  });

  it("says so when nothing has been pushed", async () => {
    fetchMock.mockImplementation(async () => json({ head: null, files: [], truncated: false }));
    render(PlayerFilesView, { props: { ...props, revision: null } });
    expect(await screen.findByText(/Nothing pushed yet/)).toBeInTheDocument();
  });

  it("re-reads the tree when a new commit lands", async () => {
    const { rerender } = render(PlayerFilesView, { props });
    await screen.findByTestId("files-content");
    const listings = () =>
      fetchMock.mock.calls.filter(([u]) => new URL(String(u), "http://localhost").pathname === BASE)
        .length;
    expect(listings()).toBe(1);
    await rerender({ ...props, revision: "b".repeat(40) });
    await waitFor(() => expect(listings()).toBe(2));
  });
});
