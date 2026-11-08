import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import SessionLinksCard from "./SessionLinksCard.svelte";
import type { SessionLinks } from "$lib/api";
import { ApiError } from "$lib/api/errors";
import { notify } from "$lib/notifications.svelte";

const linkSessionMock = vi.fn();
const unlinkSessionMock = vi.fn();
vi.mock("$lib/api", () => ({
  linkSession: (...args: unknown[]) => linkSessionMock(...args),
  unlinkSession: (...args: unknown[]) => unlinkSessionMock(...args),
}));
vi.mock("$lib/notifications.svelte", () => ({
  notify: { success: vi.fn(), error: vi.fn() },
}));

function group(all_finished: boolean): SessionLinks {
  return {
    group_id: "g",
    all_finished,
    sessions: [
      {
        id: "s1",
        join_code: "AAA111",
        name: "Round 1",
        status: all_finished ? "finished" : "running",
        project_id: "p",
        project_name: "FizzBuzz",
        project_slug: "fizzbuzz",
        created_at: "2026-10-31T10:00:00Z",
        started_at: null,
        finished_at: null,
        player_count: 2,
        current: true,
      },
      {
        id: "s2",
        join_code: "BBB222",
        name: "Round 2",
        status: all_finished ? "finished" : "lobby",
        project_id: "p",
        project_name: "FizzBuzz",
        project_slug: "fizzbuzz",
        created_at: "2026-10-31T10:05:00Z",
        started_at: null,
        finished_at: null,
        player_count: 1,
        current: false,
      },
    ],
    leaderboard: [
      {
        user_id: "u-ada",
        display_name: "Ada",
        username: "ada",
        avatar_url: null,
        total_points: 70,
        sessions_played: 2,
        per_session: [
          { session_id: "s1", points: 30, rank: 2 },
          { session_id: "s2", points: 40, rank: 1 },
        ],
      },
      {
        user_id: null,
        display_name: "Guest",
        total_points: 50,
        sessions_played: 1,
        per_session: [{ session_id: "s1", points: 50, rank: 1 }],
      },
    ],
  };
}

const base = { sessionId: "s1", joinCode: "AAA111" };

describe("SessionLinksCard", () => {
  afterEach(() => {
    cleanup();
    linkSessionMock.mockReset();
    unlinkSessionMock.mockReset();
  });

  it("renders nothing for a spectator of an unlinked session", () => {
    render(SessionLinksCard, { props: { ...base, links: null } });
    expect(screen.queryByTestId("session-links")).toBeNull();
  });

  it("offers the host a way to start a group", () => {
    render(SessionLinksCard, { props: { ...base, links: null, canControl: true } });
    expect(screen.getByTestId("session-links-empty")).toBeTruthy();
    expect(screen.getByTestId("session-links-form")).toBeTruthy();
    expect(screen.queryByText("Unlink this session")).toBeNull();
  });

  it("lists the group with this session marked and the others linked", () => {
    render(SessionLinksCard, { props: { ...base, links: group(false) } });
    const me = screen.getByTestId("session-link-AAA111");
    expect(me.getAttribute("data-current")).toBe("true");
    expect(me.textContent).toContain("This session");
    expect(me.querySelector("a")).toBeNull();
    const other = screen.getByTestId("session-link-BBB222");
    expect(other.querySelector("a")?.getAttribute("href")).toBe("/s/BBB222");
    expect(other.textContent).toContain("Lobby");
    expect(other.textContent).toContain("1 player");
    // Not over yet: the standings are live, and say so.
    expect(screen.getByTestId("session-links-standings")).toBeTruthy();
    expect(screen.getByTestId("session-links-standings-state").textContent).toContain("Live");
    // A spectator has no controls.
    expect(screen.queryByTestId("session-links-form")).toBeNull();
  });

  it("lists every room's players by points once every session has finished", () => {
    render(SessionLinksCard, { props: { ...base, links: group(true) } });
    const rows = screen.getAllByTestId("session-links-row");
    expect(rows).toHaveLength(2);
    expect(rows[0].textContent).toContain("Ada");
    expect(rows[0].textContent).toContain("70");
    expect(rows[0].querySelector("a")?.getAttribute("href")).toBe("/u/ada");
    // Ada sat in both rounds: each named with its points; the other round links.
    const origins = screen.getAllByTestId("session-links-origin");
    expect(origins[0].textContent?.replace(/\s+/g, " ").trim()).toBe("Round 1 30 · Round 2 40");
    expect(origins[0].querySelector("a")?.getAttribute("href")).toBe("/s/BBB222");
    // The guest played round 1 only — the session, no breakdown.
    expect(rows[1].textContent).toContain("Guest");
    expect(origins[1].textContent?.trim()).toBe("Round 1");
    expect(screen.getByTestId("session-links-standings-state").textContent).toContain(
      "Every session has finished",
    );
  });

  it("links by join code and hands the new group back", async () => {
    const next = group(false);
    linkSessionMock.mockResolvedValue(next);
    const onChanged = vi.fn();
    render(SessionLinksCard, { props: { ...base, links: null, canControl: true, onChanged } });
    const input = screen.getByPlaceholderText("ABC123") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "bbb222" } });
    await fireEvent.submit(screen.getByTestId("session-links-form"));
    await waitFor(() => expect(onChanged).toHaveBeenCalledWith(next));
    expect(linkSessionMock).toHaveBeenCalledWith("s1", "BBB222", expect.anything());
  });

  it("says why a link was refused: another project", async () => {
    linkSessionMock.mockRejectedValue(new ApiError(409, "link_across_projects", {}));
    const onChanged = vi.fn();
    render(SessionLinksCard, { props: { ...base, links: null, canControl: true, onChanged } });
    const input = screen.getByPlaceholderText("ABC123") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "WTHR01" } });
    await fireEvent.submit(screen.getByTestId("session-links-form"));
    await waitFor(() =>
      expect(notify.error).toHaveBeenCalledWith(
        "Failed to link: linked sessions run the same project — that session runs another one.",
      ),
    );
    expect(onChanged).not.toHaveBeenCalled();
  });

  it("unlinks after a confirmation and hands back null", async () => {
    unlinkSessionMock.mockResolvedValue(undefined);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const onChanged = vi.fn();
    render(SessionLinksCard, {
      props: { ...base, links: group(false), canControl: true, onChanged },
    });
    await fireEvent.click(screen.getByText("Unlink this session"));
    await waitFor(() => expect(onChanged).toHaveBeenCalledWith(null));
    expect(unlinkSessionMock).toHaveBeenCalledWith("s1", expect.anything());
    confirm.mockRestore();
  });
});
