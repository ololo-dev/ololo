import { afterEach, describe, expect, it, vi } from "vitest";
import { render, fireEvent, cleanup, within } from "@testing-library/svelte";
import LiveSessions from "./LiveSessions.svelte";
import type { Session } from "$lib/api";
import type { WsProjectClient } from "$lib/ws-project.svelte";

function session(over: Partial<Session> = {}): Session {
  return {
    id: "s-1",
    name: "Friday Night Showdown",
    status: "running",
    owner_id: null,
    project_id: "p-1",
    created_at: "2026-07-31T00:00:00Z",
    join_code: "LIVE01",
    ...over,
  };
}

/** Only `sessionCountdowns` is read by this component. */
function clientWith(countdowns: Record<string, { type: "lobby" | "running"; secs: number }>) {
  return { sessionCountdowns: countdowns } as unknown as WsProjectClient;
}

function renderStrip(sessions: Session[], wsClient: WsProjectClient | null = null) {
  return render(LiveSessions, { props: { sessions, wsClient } });
}

describe("LiveSessions", () => {
  afterEach(cleanup);

  it("renders nothing when there is nothing live", () => {
    const { queryByTestId } = renderStrip([]);
    expect(queryByTestId("live-sessions")).toBeNull();
  });

  it("shows the running clock once a countdown tick arrives", () => {
    const { getByTestId } = renderStrip(
      [session()],
      clientWith({ "s-1": { type: "running", secs: 125 } }),
    );
    const card = getByTestId("live-session");
    expect(within(card).getByText("Ends in")).not.toBeNull();
    expect(within(card).getByText("00:02:05")).not.toBeNull();
  });

  it("labels a lobby countdown as time until start", () => {
    const { getByTestId } = renderStrip(
      [session({ status: "lobby" })],
      clientWith({ "s-1": { type: "lobby", secs: 30 } }),
    );
    const card = getByTestId("live-session");
    expect(within(card).getByText("Starts in")).not.toBeNull();
    expect(within(card).getByText("00:00:30")).not.toBeNull();
  });

  it("omits the clock until the server sends one", () => {
    // The countdown is server-driven; nothing is invented client-side.
    const { getByTestId } = renderStrip([session()]);
    const card = getByTestId("live-session");
    expect(within(card).queryByText("Ends in")).toBeNull();
    expect(within(card).queryByText("Starts in")).toBeNull();
  });

  it("copies the whole join command, since joining is terminal-only", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    const { getByTestId } = renderStrip([session()]);
    const card = getByTestId("live-session");

    expect(within(card).getByText("ololo join LIVE01")).not.toBeNull();
    await fireEvent.click(within(card).getByLabelText("Copy to clipboard"));
    expect(writeText).toHaveBeenCalledWith("ololo join LIVE01");
    expect(within(card).getByLabelText("Copied!")).not.toBeNull();
  });

  it("links to the session as a spectator, not as a way to join", () => {
    const { getByTestId } = renderStrip([session()]);
    const card = getByTestId("live-session");
    const link = card.querySelector('a[href="/s/LIVE01"]');
    expect(link).not.toBeNull();
    expect(link?.textContent?.trim()).toBe("Watch live →");
  });

  it("pluralizes the player count and treats a missing count as zero", () => {
    const { getAllByTestId } = renderStrip([
      session({ id: "a", player_count: 1 }),
      session({ id: "b", player_count: 4 }),
      session({ id: "c" }),
    ]);
    const cards = getAllByTestId("live-session");
    expect(within(cards[0]).getByText("1 player")).not.toBeNull();
    expect(within(cards[1]).getByText("4 players")).not.toBeNull();
    expect(within(cards[2]).getByText("0 players")).not.toBeNull();
  });

  it("renders a session with no join code without a command or CTA", () => {
    const { getByTestId } = renderStrip([session({ join_code: null })]);
    const card = getByTestId("live-session");
    expect(within(card).getByText("Friday Night Showdown")).not.toBeNull();
    expect(card.querySelector("a")).toBeNull();
    expect(within(card).queryByText(/ololo join/)).toBeNull();
  });
});
