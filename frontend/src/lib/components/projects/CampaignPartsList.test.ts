import { describe, it, expect } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/svelte";
import CampaignPartsList from "./CampaignPartsList.svelte";
import type { ProjectPart, ProjectPartState } from "$lib/api";

// A part is a full project summary plus its per-caller state, because the
// campaign page renders the catalog's own card for it.
function part(over: Partial<ProjectPart> & { id: string; name: string }): ProjectPart {
  return {
    owner_user_id: "22222222-2222-2222-2222-222222222222",
    public: true,
    archived_at: null,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    slug: over.name.toLowerCase().replace(/\s+/g, "-"),
    description: "A part of the campaign.",
    has_active_sessions: false,
    category: "Reinvent the Wheel",
    tags: [],
    cover_image_url: null,
    part_ordinal: 0,
    task_count: 8,
    points: { value: 10, fail: -5, no_response: -10, completion_bonus: 10 },
    points_range: null,
    intervals: {
      deadline_secs: 60,
      min_interval_secs: 5,
      interval_increment_secs: 5,
      max_interval_secs: 60,
    },
    session_duration_secs: 900,
    idle_timeout_secs: 300,
    show_tasks: true,
    state: "available" as ProjectPartState,
    ...over,
  } as ProjectPart;
}

const parts: ProjectPart[] = [
  part({ id: "p1", name: "Speak SQL", part_ordinal: 0, state: "completed" }),
  part({ id: "p2", name: "Data That Stays", part_ordinal: 1, state: "available" }),
  part({ id: "p3", name: "Order and Speed", part_ordinal: 2, state: "locked" }),
];

describe("CampaignPartsList", () => {
  it("opens on cards and can switch to the list", async () => {
    render(CampaignPartsList, { parts, signedIn: true });

    expect(screen.getByRole("button", { name: "Cards" }).getAttribute("aria-pressed")).toBe("true");
    // Cards are the catalog's card: a cover badge per part, no numbered rows.
    expect(screen.getAllByTestId("project-part-state")).toHaveLength(parts.length);

    await fireEvent.click(screen.getByRole("button", { name: "List" }));
    expect(screen.getByRole("button", { name: "List" }).getAttribute("aria-pressed")).toBe("true");
    expect(screen.queryAllByTestId("project-part-state")).toHaveLength(0);
    // Every part is still reachable in either layout.
    expect(screen.getByText("Speak SQL")).not.toBeNull();
    expect(screen.getByText("Order and Speed")).not.toBeNull();
  });

  it("links every part, locked ones included", () => {
    render(CampaignPartsList, { parts, signedIn: true });
    const links = screen.getAllByRole("link").map((a) => a.getAttribute("href"));
    expect(links).toContain("/projects/speak-sql");
    expect(links).toContain("/projects/order-and-speed");
  });

  it("tells a locked part what unlocks it, in the view with room to say it", async () => {
    render(CampaignPartsList, { parts, signedIn: true });
    // The card carries the badge alone; the list spells out the route.
    await fireEvent.click(screen.getByRole("button", { name: "List" }));
    expect(screen.getAllByText(/Finish “Data That Stays” first/).length).toBeGreaterThan(0);
  });

  it("shows each part's state", () => {
    render(CampaignPartsList, { parts, signedIn: true });
    expect(screen.getByText("Completed")).not.toBeNull();
    expect(screen.getByText("Ready to play")).not.toBeNull();
    expect(screen.getByText("Locked")).not.toBeNull();
  });

  it("offers Start on the parts that can be played, and on no others", async () => {
    const started: string[] = [];
    render(CampaignPartsList, {
      parts,
      signedIn: true,
      onStart: (slug: string) => started.push(slug),
    });

    // Cards: the catalog's own button, on the open part and the cleared one
    // (a campaign part may be replayed), never on the locked one.
    const buttons = screen.getAllByRole("button", { name: "Start session" });
    expect(buttons).toHaveLength(2);
    await fireEvent.click(buttons[0]);
    expect(started).toEqual(["speak-sql"]);

    // The list layout carries the same affordance on the same parts.
    await fireEvent.click(screen.getByRole("button", { name: "List" }));
    expect(screen.getByTestId("campaign-start-1")).not.toBeNull();
    expect(screen.queryByTestId("campaign-start-2")).toBeNull();
    await fireEvent.click(screen.getByTestId("campaign-start-1"));
    expect(started).toEqual(["speak-sql", "data-that-stays"]);
  });

  it("offers no Start at all when the page cannot open the popup", () => {
    render(CampaignPartsList, { parts, signedIn: true });
    expect(screen.queryAllByRole("button", { name: "Start session" })).toHaveLength(0);
  });

  it("invites an anonymous visitor to sign in for their own progress", () => {
    render(CampaignPartsList, { parts, signedIn: false });
    expect(screen.getByText(/Sign in to see how far you have got/)).not.toBeNull();
  });

  it("says so when a campaign has no parts yet", () => {
    render(CampaignPartsList, { parts: [], signedIn: true });
    expect(screen.getByText("This campaign has no parts yet.")).not.toBeNull();
    expect(screen.queryByRole("button", { name: "Cards" })).toBeNull();
  });

  it("keeps a card's teaser free of markdown table syntax", () => {
    const withTable = [
      part({
        id: "p9",
        name: "Contract Part",
        description: "Intro line.\n\n| Statement | Output |\n|---|---|\n| `X` | `Y` |\n",
      }),
    ];
    render(CampaignPartsList, { parts: withTable, signedIn: true });
    const card = screen.getByRole("link");
    expect(within(card).getByText(/Intro line\./)).not.toBeNull();
    expect(card.textContent).not.toContain("|");
  });
});
