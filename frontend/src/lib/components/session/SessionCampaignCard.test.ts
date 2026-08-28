import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/svelte";
import SessionCampaignCard from "./SessionCampaignCard.svelte";
import type { SessionCampaign } from "$lib/api";

function campaign(status = "running"): SessionCampaign {
  return {
    id: "c1",
    name: "Handmade PostgreSQL",
    slug: "handmade-postgresql",
    current_part_ordinal: 1,
    session_status: status,
    parts: [
      {
        project_id: "p0",
        name: "The Server",
        slug: "handmade-postgresql-1",
        part_ordinal: 0,
        current: false,
        cleared_by: [
          {
            user_id: "u1",
            display_name: "Ada",
            join_code: "AAA111",
            finished_at: "2026-08-20T10:00:00Z",
          },
        ],
      },
      {
        project_id: "p1",
        name: "The SQL Engine",
        slug: "handmade-postgresql-2",
        part_ordinal: 1,
        current: true,
        cleared_by: [],
      },
      {
        project_id: "p2",
        name: "The Storage Engine",
        slug: "handmade-postgresql-3",
        part_ordinal: 2,
        current: false,
        cleared_by: [],
      },
    ],
  };
}

describe("SessionCampaignCard", () => {
  it("Links a cleared part to the session it was cleared in", () => {
    render(SessionCampaignCard, { campaign: campaign() });

    const cleared = screen.getByTestId("campaign-cleared-0-u1");
    expect(cleared.getAttribute("href")).toBe("/s/AAA111");
    expect(cleared.textContent).toContain("Ada");
  });

  it("Says where the session sits and which part is being played", () => {
    render(SessionCampaignCard, { campaign: campaign() });

    expect(screen.getByTestId("session-campaign").textContent).toContain("Part 2 of 3");
    expect(screen.getByTestId("campaign-part-1").textContent).toContain("Playing now");
    // A part nobody has reached is "Ahead", not a failure to clear it.
    expect(screen.getByTestId("campaign-part-2").textContent).toContain("Ahead");
  });

  it("Stops claiming the part is being played once the session is over", () => {
    // "Playing now" on a finished session is a lie about the present tense;
    // what the reader wants is what came of it.
    const ended = campaign("finished");
    render(SessionCampaignCard, { campaign: ended });
    expect(screen.getByTestId("campaign-part-1").textContent).toContain("Not cleared");
    expect(screen.getByTestId("campaign-part-1").textContent).not.toContain("Playing now");
  });

  it("Says the part was cleared here when this session finished it", () => {
    const ended = campaign("finished");
    ended.parts[1].cleared_by = [
      { user_id: "u1", display_name: "Ada", join_code: "BBB222", finished_at: null },
    ];
    render(SessionCampaignCard, { campaign: ended });

    expect(screen.getByTestId("campaign-part-1").textContent).toContain("Cleared here");
    // The chip for this very session is not a link to the page you are on.
    expect(screen.getByTestId("campaign-cleared-1-u1").tagName).toBe("SPAN");
  });

  it("Names a cancelled session rather than blaming the player", () => {
    render(SessionCampaignCard, { campaign: campaign("cancelled") });
    expect(screen.getByTestId("campaign-part-1").textContent).toContain("Session cancelled");
  });

  it("Marks an earlier part nobody in the room cleared", () => {
    const c = campaign();
    c.parts[0].cleared_by = [];
    render(SessionCampaignCard, { campaign: c });

    expect(screen.getByTestId("campaign-part-0").textContent).toContain("Not cleared");
  });
});
