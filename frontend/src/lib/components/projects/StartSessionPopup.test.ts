import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, cleanup, fireEvent, waitFor } from "@testing-library/svelte";

const hoisted = vi.hoisted(() => ({
  page: { data: {} as Record<string, unknown> },
  getProjectBySlug: vi.fn(),
  createSession: vi.fn(),
  listProjectSessions: vi.fn(),
  patchSession: vi.fn(),
}));

vi.mock("$app/state", () => ({ page: hoisted.page }));
vi.mock("$lib/api", async (orig) => ({
  ...(await orig<typeof import("$lib/api")>()),
  getProjectBySlug: hoisted.getProjectBySlug,
  createSession: hoisted.createSession,
  listProjectSessions: hoisted.listProjectSessions,
  patchSession: hoisted.patchSession,
}));

import StartSessionPopup from "./StartSessionPopup.svelte";
import { ApiError } from "$lib/api";

const ME = "u-admin";
function session(over: Record<string, unknown>) {
  return {
    id: `s-${over.join_code}`,
    name: "Extreme Startup",
    status: "lobby",
    owner_id: ME,
    project_id: "p-1",
    created_at: "2026-10-25T10:00:00Z",
    player_count: 0,
    ...over,
  };
}

function asAdmin() {
  hoisted.page.data = { isAdmin: true, user: { id: ME } };
}

/** Render as an admin and switch to the hosting tab. */
async function openHosting(slug = "extreme-startup") {
  asAdmin();
  const r = render(StartSessionPopup, { slug, open: true });
  await fireEvent.click(r.getByRole("tab", { name: "Host several sessions" }));
  return r;
}

const rows = (c: HTMLElement) =>
  [...c.querySelectorAll('[data-testid="hosted-session"]')].map((li) =>
    (li.textContent ?? "").replace(/\s+/g, " ").trim(),
  );

describe("StartSessionPopup", () => {
  beforeEach(() => {
    hoisted.page.data = { isAdmin: false, user: { id: "u-player" } };
    for (const f of [
      hoisted.getProjectBySlug,
      hoisted.createSession,
      hoisted.listProjectSessions,
      hoisted.patchSession,
    ])
      f.mockReset();
    hoisted.getProjectBySlug.mockResolvedValue({ id: "p-1", name: "Extreme Startup" });
    hoisted.listProjectSessions.mockResolvedValue([]);
  });
  afterEach(() => cleanup());

  it("renders the start command when open with a slug", () => {
    const { getByText, getByRole, queryByRole } = render(StartSessionPopup, {
      slug: "my-project",
      open: true,
    });
    expect(getByRole("dialog", { name: "Start session" })).toBeTruthy();
    expect(getByText("ololo start my-project")).toBeTruthy();
    expect(queryByRole("tablist")).toBeNull();
  });

  it("does not render when closed", () => {
    const { queryByRole } = render(StartSessionPopup, { slug: "my-project", open: false });
    expect(queryByRole("dialog")).toBeNull();
  });

  it("does not render without a slug even when open", () => {
    const { queryByRole } = render(StartSessionPopup, { slug: "", open: true });
    expect(queryByRole("dialog")).toBeNull();
  });

  it("opens an admin on the console too, with hosting several sessions a tab away", async () => {
    asAdmin();
    const { getByRole, queryByTestId, getByText, rerender } = render(StartSessionPopup, {
      slug: "extreme-startup",
      open: true,
    });
    expect(getByText("ololo start extreme-startup")).toBeTruthy();
    expect(queryByTestId("host-from-web")).toBeNull();
    expect(getByRole("tab", { name: "Play from the console" }).getAttribute("aria-selected")).toBe(
      "true",
    );

    await fireEvent.click(getByRole("tab", { name: "Host several sessions" }));
    expect(queryByTestId("host-from-web")).not.toBeNull();

    // Closing and reopening lands on the console again.
    await rerender({ slug: "extreme-startup", open: false });
    await rerender({ slug: "extreme-startup", open: true });
    expect(queryByTestId("host-from-web")).toBeNull();
    expect(getByText("ololo start extreme-startup")).toBeTruthy();
  });

  it("lists the admin's own live sessions of the project, from the server", async () => {
    hoisted.listProjectSessions.mockResolvedValue([
      session({
        join_code: "BBBBBB",
        status: "running",
        player_count: 3,
        created_at: "2026-10-25T10:05:00Z",
      }),
      session({ join_code: "AAAAAA", player_count: 1 }),
      session({ join_code: "OTHERS", owner_id: "u-someone" }),
      session({ join_code: "DONE00", status: "finished" }),
    ]);
    const { container, findAllByTestId, getByRole } = await openHosting("extreme-startup");

    await findAllByTestId("hosted-session");
    expect(rows(container)).toEqual([
      "AAAAAA Lobby 1 player Copy command Open ↗ Cancel session",
      "BBBBBB Running 3 players Copy command Open ↗ Cancel session",
    ]);
    const links = [...container.querySelectorAll("a")].map((a) => a.getAttribute("href"));
    expect(links).toEqual(["/s/AAAAAA/lobby", "/s/BBBBBB"]);
    expect(getByRole("button", { name: "+ Create another session" })).toBeTruthy();
  });

  it("creates sessions named after the project, and the new one shows up", async () => {
    const created: ReturnType<typeof session>[] = [];
    hoisted.listProjectSessions.mockImplementation(async () => [...created]);
    hoisted.createSession.mockImplementation(async (name: string) => {
      const s = session({ join_code: created.length === 0 ? "AAAAAA" : "BBBBBB", name });
      created.push(s);
      return s;
    });
    const { container, findByRole, getByRole, queryByRole } = await openHosting("extreme-startup");
    expect(queryByRole("textbox")).toBeNull();

    await fireEvent.click(await findByRole("button", { name: "Create session" }));
    await fireEvent.click(await findByRole("button", { name: "+ Create another session" }));

    await waitFor(() => expect(rows(container)).toHaveLength(2));
    expect(hoisted.createSession).toHaveBeenNthCalledWith(1, "Extreme Startup", "p-1");
    expect(hoisted.createSession).toHaveBeenNthCalledWith(2, "Extreme Startup", "p-1");
    expect(getByRole("button", { name: "Copy all join commands" })).toBeTruthy();
  });

  it("copies every join command at once", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    hoisted.listProjectSessions.mockResolvedValue([
      session({ join_code: "AAAAAA" }),
      session({ join_code: "BBBBBB", created_at: "2026-10-25T10:01:00Z" }),
    ]);
    const { findByRole } = await openHosting("extreme-startup");

    await fireEvent.click(await findByRole("button", { name: "Copy all join commands" }));
    expect(writeText).toHaveBeenCalledWith("ololo join AAAAAA\nololo join BBBBBB");
    await findByRole("button", { name: "Copied ✓" });
  });

  it("cancels a session on a second click, without a browser dialog", async () => {
    let list = [session({ join_code: "AAAAAA" })];
    hoisted.listProjectSessions.mockImplementation(async () => list);
    hoisted.patchSession.mockImplementation(async () => {
      list = [];
      return {};
    });
    const confirmSpy = vi.spyOn(window, "confirm");
    const { findByRole, findByText } = await openHosting("extreme-startup");

    await fireEvent.click(await findByRole("button", { name: "Cancel session" }));
    expect(hoisted.patchSession).not.toHaveBeenCalled();
    await fireEvent.click(await findByRole("button", { name: "Click again to cancel" }));

    expect(hoisted.patchSession).toHaveBeenCalledWith("s-AAAAAA", { status: "cancelled" });
    await findByText("No live sessions of this project yet.");
    expect(confirmSpy).not.toHaveBeenCalled();
  });

  it("says why the server refused", async () => {
    hoisted.createSession.mockRejectedValue(new ApiError(409, "campaign_project", null));
    const { findByRole, queryAllByTestId } = await openHosting("campaign");
    await fireEvent.click(await findByRole("button", { name: "Create session" }));
    expect((await findByRole("alert")).textContent).toContain("start one of its parts");
    expect(queryAllByTestId("hosted-session")).toHaveLength(0);
  });
});
