import { describe, it, expect } from "vitest";
import { render, screen, within, fireEvent } from "@testing-library/svelte";
import Page from "./+page.svelte";
import type { Project } from "$lib/api";

/**
 * The catalog lists standalone projects and campaigns — never the parts
 * inside a campaign. Parts shipped to the grid once (five chapters of
 * Handmade PostgreSQL beside every other project), which is what these tests
 * pin: a part is reached through its campaign's page, and the campaign card
 * says how many parts it holds.
 */

const OWNER = "22222222-2222-2222-2222-222222222222";

function project(over: Partial<Project> & { id: string; name: string }): Project {
  return {
    owner_user_id: OWNER,
    public: true,
    archived_at: null,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    description: "",
    slug: over.name.toLowerCase().replace(/\s+/g, "-"),
    has_active_sessions: false,
    category: "Reinvent the Wheel",
    tags: [],
    cover_image_url: null,
    task_count: 3,
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
    ...over,
  } as Project;
}

const campaign = project({
  id: "aaaaaaaa-0000-0000-0000-000000000000",
  name: "Handmade PostgreSQL",
  task_count: 0,
  part_count: 2,
  // Its own duration is the untouched default; the parts are 15 + 20 min.
  session_duration_secs: 3600,
  parts_duration_secs: 900 + 1200,
});

const partOne = project({
  id: "bbbbbbbb-0000-0000-0000-000000000000",
  name: "Handmade PostgreSQL Part One",
  parent_project_id: campaign.id,
  part_ordinal: 0,
  parent_project_slug: campaign.slug,
});

const standalone = project({
  id: "cccccccc-0000-0000-0000-000000000000",
  name: "Fizzbuzz Game",
  category: "Code Golf",
});

const baseData = {
  projects: [campaign, partOne, standalone],
  categories: ["Reinvent the Wheel", "Code Golf"],
  currentUserId: null,
  isAuthenticated: false,
  isAdmin: false,
  allowProjectCreation: false,
  replayEnabled: false,
  user: null,
};

describe("routes/projects/+page.svelte", () => {
  it("lists the campaign and standalone projects", () => {
    render(Page, { data: baseData });
    expect(screen.getByText("Handmade PostgreSQL")).not.toBeNull();
    expect(screen.getByText("Fizzbuzz Game")).not.toBeNull();
  });

  it("keeps campaign parts out of the grid", () => {
    render(Page, { data: baseData });
    expect(screen.queryByText("Handmade PostgreSQL Part One")).toBeNull();
  });

  it("marks the campaign with its part count", () => {
    render(Page, { data: baseData });
    const badges = screen.getAllByTestId("project-campaign-parts");
    expect(badges).toHaveLength(1);
    expect(badges[0].textContent).toContain("2 parts");
  });

  it("does not offer to start a campaign — the server refuses that", () => {
    // `ololo start handmade-postgresql` is rejected with campaign_project:
    // a campaign hosts no sessions, its parts do. The card opens it instead.
    render(Page, { data: baseData });

    const cards = screen.getAllByRole("link");
    const campaignCard = cards.find((c) => c.textContent?.includes("Handmade PostgreSQL"));
    expect(campaignCard).toBeDefined();
    expect(within(campaignCard!).queryByRole("button", { name: /Start session/ })).toBeNull();

    // A standalone project still offers it.
    const plainCard = cards.find((c) => c.textContent?.includes("Fizzbuzz Game"));
    expect(within(plainCard!).getByRole("button", { name: /Start session/ })).not.toBeNull();
  });
  it("shows a campaign's total playing time, not its own unused default", () => {
    // A campaign runs no sessions, so its session_duration_secs is a default
    // nobody plays; the card has to answer "how long is all of this".
    render(Page, { data: baseData });
    const cards = screen.getAllByRole("link");
    const campaignCard = cards.find((c) => c.textContent?.includes("Handmade PostgreSQL"))!;
    expect(campaignCard.textContent).toContain("35 min");
    expect(campaignCard.textContent).not.toContain("1 h");

    const plainCard = cards.find((c) => c.textContent?.includes("Fizzbuzz Game"))!;
    expect(plainCard.textContent).toContain("15 min");
  });

  it("filters the catalog down to campaigns, and to single sessions", async () => {
    // Two different commitments — an evening against a series played part by
    // part — and no way to ask for either until now.
    render(Page, { data: baseData });

    await fireEvent.click(screen.getByTestId("shape-filter-campaign"));
    expect(screen.getByText("Handmade PostgreSQL")).not.toBeNull();
    expect(screen.queryByText("Fizzbuzz Game")).toBeNull();

    await fireEvent.click(screen.getByTestId("shape-filter-single"));
    expect(screen.getByText("Fizzbuzz Game")).not.toBeNull();
    expect(screen.queryByText("Handmade PostgreSQL")).toBeNull();

    // Pressing the active one again clears it.
    await fireEvent.click(screen.getByTestId("shape-filter-single"));
    expect(screen.getByText("Handmade PostgreSQL")).not.toBeNull();
    expect(screen.getByText("Fizzbuzz Game")).not.toBeNull();
  });

  it("offers no shape switch when the shelf holds only one shape", () => {
    render(Page, { data: { ...baseData, projects: [standalone] } });
    expect(screen.queryByTestId("shape-filter-campaign")).toBeNull();
    expect(screen.queryByTestId("shape-filter-single")).toBeNull();
  });
});
