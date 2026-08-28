import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/svelte";
import CampaignPartNav from "./CampaignPartNav.svelte";
import type { ProjectPart, ProjectPartState } from "$lib/api";

/**
 * A part page stays an ordinary project page — its own tasks, sessions and
 * board. This strip is the only campaign furniture on it, so it has to carry
 * the whole story: where the part sits, and the step either way.
 */

function part(over: Partial<ProjectPart> & { id: string; name: string }): ProjectPart {
  return {
    owner_user_id: "22222222-2222-2222-2222-222222222222",
    public: true,
    archived_at: null,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    slug: over.name.toLowerCase().replace(/\s+/g, "-"),
    description: "",
    has_active_sessions: false,
    category: null,
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

const base = {
  parts,
  campaignSlug: "handmade-postgresql",
  campaignName: "Handmade PostgreSQL",
};

describe("CampaignPartNav", () => {
  it("places the part in its campaign and links back to it", () => {
    render(CampaignPartNav, { ...base, currentId: "p2" });
    expect(screen.getByText("Part 2 of 3")).not.toBeNull();
    const campaign = screen.getByRole("link", { name: "Handmade PostgreSQL" });
    expect(campaign.getAttribute("href")).toBe("/projects/handmade-postgresql");
  });

  it("steps both ways", () => {
    render(CampaignPartNav, { ...base, currentId: "p2" });
    expect(screen.getByText("Speak SQL").closest("a")?.getAttribute("href")).toBe(
      "/projects/speak-sql",
    );
    expect(screen.getByText("Order and Speed").closest("a")?.getAttribute("href")).toBe(
      "/projects/order-and-speed",
    );
  });

  it("offers no previous on the first part", () => {
    render(CampaignPartNav, { ...base, currentId: "p1" });
    expect(screen.queryByText(/← Previous/)).toBeNull();
    expect(screen.getByText(/Next/)).not.toBeNull();
  });

  it("says so on the last part instead of pointing nowhere", () => {
    render(CampaignPartNav, { ...base, currentId: "p3" });
    expect(screen.getByText("Last part of the campaign")).not.toBeNull();
    expect(screen.getByText(/← Previous/)).not.toBeNull();
  });

  it("marks a next part the player has not unlocked", () => {
    render(CampaignPartNav, { ...base, currentId: "p2" });
    expect(screen.getByText(/Next · locked/)).not.toBeNull();
  });

  it("renders nothing for a project that is not in this campaign", () => {
    const { container } = render(CampaignPartNav, { ...base, currentId: "stranger" });
    expect(container.querySelector("nav")).toBeNull();
  });
});
