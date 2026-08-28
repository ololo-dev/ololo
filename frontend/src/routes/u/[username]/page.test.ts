import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/svelte";
import Page from "./+page.svelte";

const SESSIONS = [
  {
    session_id: "s1",
    name: "Hop-Hop Game",
    session_datetime: "2026-08-03T10:06:00Z",
    participant_count: 1,
    status: "finished",
    join_code: "2NVOG2",
    project_id: "p1",
    project_name: "Hop-Hop Game",
    project_slug: "hop-hop-game",
    game_points: 390,
    placement: 1,
    agent: "opencode",
    models: [],
  },
  {
    session_id: "s2",
    name: "Repeat Each Character",
    session_datetime: "2026-08-03T11:00:00Z",
    participant_count: 3,
    status: "running",
    join_code: "V57RLS",
    project_id: "p2",
    project_name: "Repeat Each Character",
    project_slug: "repeat-each-character",
    game_points: 12,
    placement: null,
    agent: "claude",
    models: [],
  },
];

function data(
  sessions: unknown[] = SESSIONS,
  paging: { total?: number; page?: number; per_page?: number } = {},
) {
  return {
    profile: {
      username: "gentle-teal-rose",
      display_name: "Anod",
      avatar_url: null,
      joined_at: "2026-07-29T00:00:00Z",
    },
    sessions: {
      sessions,
      total: paging.total ?? sessions.length,
      page: paging.page ?? 1,
      per_page: paging.per_page ?? 20,
    },
  };
}

describe("public profile sessions", () => {
  it("Says where the session link goes instead of showing a join code", () => {
    render(Page, { data: data() as never });
    // The join code was the link, and read as a serial number rather than as
    // a way into the session — nothing on the row said it was clickable.
    const link = document.querySelector('a[href="/s/2NVOG2"]');
    expect(link).not.toBeNull();
    expect(link?.textContent?.trim()).toContain("Results");
    expect(link?.textContent?.trim()).not.toBe("2NVOG2");
    // Short on the row, full to anyone who cannot see the column it sits in.
    expect(link?.getAttribute("aria-label")).toBe("Session results");
  });

  it("Labels a live session by what it will actually show", () => {
    render(Page, { data: data() as never });
    const link = document.querySelector('a[href="/s/V57RLS"]');
    // /s/<code> is the live board while a session runs and the report once it
    // ends; one label for both would be wrong half the time.
    expect(link?.textContent?.trim()).toContain("Live");
    expect(link?.getAttribute("aria-label")).toBe("Live board");
  });

  it("Calls a session played alone Solo instead of ranking it first of one", () => {
    render(Page, { data: data() as never });
    const table = document.querySelector("table");
    // "#1 of 1" was true on nearly every row and carried no information.
    expect(table?.textContent).toContain("Solo");
    expect(table?.textContent).not.toContain("of 1");
    // A session with company keeps its placement.
    expect(table?.textContent).toContain("of 3");
  });

  it("Offers the rest of the history instead of stopping at twenty", () => {
    // The heading counts every session; the table showed one page of them and
    // said nothing about the other 98.
    render(Page, { data: data(SESSIONS, { total: 45, page: 2 }) as never });
    const nav = screen.getByTestId("sessions-pagination");
    expect(nav.textContent).toContain("Page 2 of 3");
    expect(nav.querySelector('a[href*="page=3"]')).not.toBeNull();
    // Page one is the bare URL, not ?page=1.
    const newer = [...nav.querySelectorAll("a")].find((a) => a.textContent?.includes("Newer"));
    expect(newer?.getAttribute("href")).not.toContain("page=");
  });

  it("Leaves the pager out when the whole history is on the page", () => {
    render(Page, { data: data() as never });
    expect(screen.queryByTestId("sessions-pagination")).toBeNull();
  });

  it("Keeps the code on the row as the identifier it is", () => {
    render(Page, { data: data() as never });
    const table = document.querySelector("table");
    expect(table?.textContent).toContain("2NVOG2");
  });

  it("Still links the project name to the project", () => {
    render(Page, { data: data() as never });
    expect(document.querySelector('a[href="/projects/hop-hop-game"]')).not.toBeNull();
  });
});
