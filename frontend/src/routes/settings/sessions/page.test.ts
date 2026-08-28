import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { readable } from "svelte/store";

const goto = vi.fn();
vi.mock("$app/navigation", () => ({
  goto: (...args: unknown[]) => goto(...args),
  invalidateAll: vi.fn(async () => {}),
}));

// The page reads the current URL to merge one filter change into the others.
vi.mock("$app/stores", () => ({
  page: readable({ url: new URL("http://localhost/settings/sessions") }),
}));

const patchSession = vi.fn(async (..._args: unknown[]) => ({}));
const deleteSession = vi.fn(async (..._args: unknown[]) => ({}));
vi.mock("$lib/api", async () => {
  const actual = await vi.importActual<Record<string, unknown>>("$lib/api");
  return {
    ...actual,
    patchSession: (...args: unknown[]) => patchSession(...args),
    deleteSession: (...args: unknown[]) => deleteSession(...args),
  };
});

import Page from "./+page.svelte";
import type { AdminSession } from "$lib/api";

function session(over: Partial<AdminSession> = {}): AdminSession {
  return {
    id: "s1",
    join_code: "OD5FJA",
    name: "Friday night fizzbuzz",
    status: "running",
    project_id: "p1",
    project_name: "fizzbuzz",
    project_slug: "fizzbuzz",
    owner_id: "u1",
    owner_display_name: "Anod",
    owner_username: "gentle-teal-rose",
    player_count: 4,
    created_at: "2026-08-03T11:20:00Z",
    started_at: null,
    finished_at: null,
    cancel_reason: null,
    cancelled_by: null,
    ...over,
  };
}

function data(sessions: AdminSession[], over: Record<string, unknown> = {}) {
  return {
    sessions: { sessions, total: sessions.length, page: 1, per_page: 25 },
    projects: [{ id: "p1", name: "fizzbuzz" }],
    filters: { status: "", projectId: "", q: "" },
    ...over,
  };
}

beforeEach(() => {
  goto.mockClear();
  patchSession.mockClear();
  deleteSession.mockClear();
  vi.spyOn(window, "confirm").mockReturnValue(true);
});

describe("settings/sessions registry", () => {
  it("Lists a session the admin neither owns nor joined", () => {
    render(Page, { data: data([session()]) as never });
    const row = screen.getByTestId("session-row-OD5FJA");
    expect(row.textContent).toContain("Friday night fizzbuzz");
    expect(row.textContent).toContain("fizzbuzz");
    expect(row.textContent).toContain("Anod");
    expect(row.textContent).toContain("Running");
  });

  it("Offers Cancel only while a session can still be cancelled", () => {
    render(Page, {
      data: data([
        session({ id: "a", join_code: "AAAAAA", status: "running" }),
        session({ id: "b", join_code: "BBBBBB", status: "paused" }),
        session({ id: "c", join_code: "CCCCCC", status: "finished" }),
        session({ id: "d", join_code: "DDDDDD", status: "cancelled" }),
      ]),
    } as never);
    const cancels = (code: string) =>
      [...screen.getByTestId(`session-row-${code}`).querySelectorAll("button")].filter((b) =>
        b.textContent?.includes("Cancel"),
      ).length;
    expect(cancels("AAAAAA")).toBe(1);
    expect(cancels("BBBBBB")).toBe(1);
    // A finished session cannot transition to cancelled, so offering the
    // button would only ever produce a 409.
    expect(cancels("CCCCCC")).toBe(0);
    expect(cancels("DDDDDD")).toBe(0);
  });

  it("Cancels through the session PATCH, not a bespoke admin route", async () => {
    render(Page, { data: data([session()]) as never });
    const row = screen.getByTestId("session-row-OD5FJA");
    const cancel = [...row.querySelectorAll("button")].find((b) =>
      b.textContent?.includes("Cancel"),
    );
    await fireEvent.click(cancel!);
    expect(patchSession).toHaveBeenCalledWith("s1", { status: "cancelled" });
  });

  it("Asks before deleting, since results go with the session", async () => {
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    render(Page, { data: data([session()]) as never });
    const row = screen.getByTestId("session-row-OD5FJA");
    const del = [...row.querySelectorAll("button")].find((b) => b.textContent?.includes("Delete"));
    await fireEvent.click(del!);
    expect(confirmSpy).toHaveBeenCalled();
    expect(deleteSession).not.toHaveBeenCalled();

    confirmSpy.mockReturnValue(true);
    await fireEvent.click(del!);
    expect(deleteSession).toHaveBeenCalledWith("s1");
  });

  it("Puts filters in the URL so a filtered view survives a reload", async () => {
    render(Page, { data: data([session()]) as never });
    await fireEvent.change(screen.getByLabelText("Filter by status"), {
      target: { value: "running" },
    });
    expect(goto).toHaveBeenCalledWith("/settings/sessions?status=running", expect.anything());
  });

  it("Drops a filter from the URL rather than sending it empty", async () => {
    // The server rejects an unparseable status instead of ignoring it, so
    // "Any status" has to remove the key, not set it to "".
    render(Page, {
      data: data([session()], { filters: { status: "running", projectId: "", q: "" } }) as never,
    });
    await fireEvent.change(screen.getByLabelText("Filter by status"), { target: { value: "" } });
    expect(goto).toHaveBeenCalledWith("/settings/sessions", expect.anything());
  });

  it("Tells an empty result apart from an empty instance", () => {
    const { unmount } = render(Page, { data: data([]) as never });
    expect(screen.getByTestId("admin-sessions").textContent).toContain(
      "No sessions have been created yet",
    );
    unmount();

    render(Page, {
      data: data([], { filters: { status: "", projectId: "", q: "nope" } }) as never,
    });
    expect(screen.getByTestId("admin-sessions").textContent).toContain(
      "No sessions match these filters",
    );
  });
});
