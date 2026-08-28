import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render } from "@testing-library/svelte";
import Page from "./+page.svelte";
import {
  makeSnapshot,
  makeTaskSummary,
  makeProbe,
  renderPage,
  type TestData,
} from "./page.test-helpers";

describe("Player detail page — snapshot", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useRealTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders player name from data.playerName", () => {
    const { getByText } = renderPage({
      live: false,
      snapshot: makeSnapshot({ display_name: "Alice Bot" }),
      token: null,
      playerName: "Alice Bot",
    });
    expect(getByText("Alice Bot")).toBeTruthy();
  });

  it("renders score from data.snapshot", () => {
    const { getByText } = renderPage({
      live: false,
      snapshot: makeSnapshot({ score: 99 }),
      token: null,
      playerName: "Test Player",
    });
    expect(getByText("99")).toBeTruthy();
  });

  it("renders rank from data.snapshot", () => {
    const { container } = renderPage({
      live: false,
      snapshot: makeSnapshot({ rank: 2 }),
      token: null,
      playerName: "Test Player",
    });
    expect(container.textContent).toContain("#2");
  });

  it("renders the account avatar image when present", () => {
    const { container } = renderPage({
      live: false,
      snapshot: makeSnapshot({
        display_name: "Alice",
        avatar_url: "https://cdn.example.com/a.png",
      }),
      token: null,
      playerName: "Alice",
    });
    const img = container.querySelector('img[alt="Alice avatar"]');
    expect(img).toBeTruthy();
    expect(img?.getAttribute("src")).toBe("https://cdn.example.com/a.png");
  });

  it("falls back to initials when there is no avatar", () => {
    const { container } = renderPage({
      live: false,
      snapshot: makeSnapshot({ display_name: "Bob Bot", avatar_url: null }),
      token: null,
      playerName: "Bob Bot",
    });
    expect(container.querySelector("img[alt$='avatar']")).toBeNull();
    // Header initials block shows "BB" for "Bob Bot".
    expect(container.textContent).toContain("BB");
  });

  it("renders an In progress badge when completion_status is in_progress", () => {
    const { getByText } = renderPage({
      live: true,
      snapshot: makeSnapshot({ completion_status: "in_progress" }),
      token: "some-token",
      playerName: "Test Player",
    });
    expect(getByText("In progress")).toBeTruthy();
  });

  it("renders an Awaiting judges badge when completion_status is awaiting_judges", () => {
    const { getByText } = renderPage({
      live: true,
      snapshot: makeSnapshot({ completion_status: "awaiting_judges" }),
      token: "some-token",
      playerName: "Test Player",
    });
    expect(getByText("Awaiting judges")).toBeTruthy();
  });

  it("renders a Completed badge when completion_status is completed", () => {
    const { getByText } = renderPage({
      live: true,
      snapshot: makeSnapshot({ completion_status: "completed" }),
      token: "some-token",
      playerName: "Test Player",
    });
    expect(getByText("Completed")).toBeTruthy();
  });

  it("renders no completion badge when completion_status is absent", () => {
    const { queryByText } = renderPage({
      live: true,
      snapshot: makeSnapshot(),
      token: "some-token",
      playerName: "Test Player",
    });
    expect(queryByText("In progress")).toBeNull();
    expect(queryByText("Awaiting judges")).toBeNull();
    expect(queryByText("Completed")).toBeNull();
  });

  it("shows Live indicator when data.live is true", () => {
    const { container } = renderPage({
      live: true,
      snapshot: makeSnapshot(),
      token: "some-token",
      playerName: "Test Player",
    });
    expect(container.textContent).toContain("Live");
  });

  it("does NOT show Live indicator when data.live is false", () => {
    const { container } = renderPage({
      live: false,
      snapshot: makeSnapshot(),
      token: null,
      playerName: "Test Player",
    });
    const liveElements = Array.from(container.querySelectorAll("*")).filter(
      (el) => el.children.length === 0 && el.textContent?.trim() === "Live",
    );
    expect(liveElements.length).toBe(0);
  });

  it("shows the next-probe countdown while the session is running", () => {
    const { container } = renderPage({
      live: true,
      snapshot: makeSnapshot({
        next_probe_at: new Date(Date.now() + 30_000).toISOString(),
      }),
      token: "some-token",
      playerName: "Test Player",
    });
    expect(container.textContent).toContain("Next probe");
  });

  it("hides the next-probe countdown once the session has ended", () => {
    // A finished session gets no further probes, so counting down to one is
    // wrong. `next_probe_at` is deliberately still in the future here: the
    // server never clears it, so the guard has to come from session state,
    // not from the timestamp having elapsed.
    const { container } = renderPage({
      live: false,
      snapshot: makeSnapshot({
        session_status: "finished",
        next_probe_at: new Date(Date.now() + 30_000).toISOString(),
      }),
      token: null,
      playerName: "Test Player",
    });
    expect(container.textContent).not.toContain("Next probe");
  });

  it("shows a compact single-line judges-scoring notice while judges settle", () => {
    // token null keeps the test from opening a real socket; the page then
    // derives sessionFinished from !data.live and renders off the snapshot.
    const { getByTestId } = renderPage({
      live: false,
      snapshot: makeSnapshot({
        session_status: "finished",
        judge_statuses: [
          {
            task_id: "t0",
            judge_slug: "golf-verify",
            judge_name: "Golf Verify",
            status: "pending",
            error: null,
            updated_at: null,
            judge_result_id: null,
          },
        ],
      }),
      token: null,
      playerName: "Test Player",
    });
    const notice = getByTestId("judges-working-dialog");
    // Compact: names the pending judge and the count, on one line — no
    // heading paragraph, no bulleted list.
    expect(notice.textContent).toContain("Judges scoring");
    expect(notice.textContent).toContain("Golf Verify");
    expect(notice.querySelector("ul")).toBeNull();
  });
});
