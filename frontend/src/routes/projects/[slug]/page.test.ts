import { describe, it, expect } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/svelte";
import SlugPage from "./+page.svelte";
import type { Session } from "$lib/api";
import type { ProjectPart } from "$lib/api";

const mockProject = {
  id: "proj-1",
  name: "Test Project",
  owner_user_id: "user-1",
  public: true,
  archived_at: null,
  created_at: "2024-01-01T00:00:00Z",
  updated_at: "2024-01-01T00:00:00Z",
  description: "A test project",
  slug: "test-project",
  has_active_sessions: false,
  task_count: 0,
  category: null,
  tags: [],
  cover_image_url: null,
  points: { value: 10, fail: -5, no_response: -10, completion_bonus: 10 },
  points_range: null,
  session_duration_secs: 3600,
  show_tasks: true,
  idle_timeout_secs: 300,
  intervals: {
    deadline_secs: 60,
    min_interval_secs: 5,
    interval_increment_secs: 5,
    max_interval_secs: 60,
  },
};

const mockSessions: Session[] = [
  {
    id: "session-1",
    project_id: "proj-1",
    name: "First Session",
    status: "running",
    owner_id: "user-1",
    join_code: "ABC123",
    created_at: "2024-01-01T00:00:00Z",
  },
];

const baseData = {
  project: mockProject,
  sessions: [] as Session[],
  judges: [],
  taskPreview: [],
  parts: [],
  topPlayers: { players: [], season_players: [], season_start: "2026-08-01T00:00:00Z" },
  message: null,
  currentUserId: null,
  isAuthenticated: false,
  isAdmin: false,
  allowProjectCreation: false,
  replayEnabled: false,
  user: null,
};

describe("routes/projects/[slug]/+page.svelte", () => {
  it("renders project name", () => {
    render(SlugPage, { data: baseData });
    const headings = document.querySelectorAll("h2");
    const heroHeading = Array.from(headings).find((el) => el.textContent?.includes("Test Project"));
    expect(heroHeading).not.toBeNull();
  });

  it("renders sessions section heading", () => {
    render(SlugPage, { data: baseData });
    const headings = document.querySelectorAll("h2");
    const sessionsHeading = Array.from(headings).find((el) => el.textContent?.includes("Sessions"));
    expect(sessionsHeading).not.toBeNull();
  });

  it("renders empty state when no sessions", async () => {
    render(SlugPage, { data: baseData });
    // Sessions live behind the Sessions tab (Top Players is the default).
    await fireEvent.click(screen.getByRole("tab", { name: /Sessions/ }));
    expect(screen.getByText("No sessions yet.")).not.toBeNull();
  });

  it("renders session rows when sessions exist", async () => {
    render(SlugPage, { data: { ...baseData, sessions: mockSessions } });
    await fireEvent.click(screen.getByRole("tab", { name: /Sessions/ }));
    // Scoped to the panel: a running session also appears in the live
    // spotlight above the tabs, so a document-wide query matches twice.
    const panel = screen.getByRole("tabpanel");
    expect(within(panel).getByText("First Session")).not.toBeNull();
  });

  it("spotlights running sessions above the tabs", () => {
    render(SlugPage, {
      data: { ...baseData, sessions: [{ ...mockSessions[0], player_count: 3 }] },
    });
    // Visible on the default (Top Players) tab — no interaction needed.
    const strip = screen.getByTestId("live-sessions");
    expect(within(strip).getByText("First Session")).not.toBeNull();
    expect(within(strip).getByText(/Live now/)).not.toBeNull();
    expect(within(strip).getByText("Running")).not.toBeNull();
    expect(within(strip).getByText("3 players")).not.toBeNull();
    // Joining is terminal-only, so the command is the primary affordance.
    expect(within(strip).getByText("ololo join ABC123")).not.toBeNull();
    expect(strip.querySelector('a[href="/s/ABC123"]')).not.toBeNull();
  });

  it("spotlights lobby sessions too, labelled as such", () => {
    const lobby: Session[] = [{ ...mockSessions[0], status: "lobby", player_count: 1 }];
    render(SlugPage, { data: { ...baseData, sessions: lobby } });
    const strip = screen.getByTestId("live-sessions");
    expect(within(strip).getByText("Lobby")).not.toBeNull();
    expect(within(strip).getByText("1 player")).not.toBeNull();
  });

  it("omits the spotlight when nothing is live", () => {
    const done: Session[] = [{ ...mockSessions[0], status: "finished" }];
    render(SlugPage, { data: { ...baseData, sessions: done } });
    expect(screen.queryByTestId("live-sessions")).toBeNull();
  });

  it("puts running sessions ahead of lobby ones", () => {
    const mixed: Session[] = [
      { ...mockSessions[0], id: "s-lobby", name: "Waiting Room", status: "lobby" },
      { ...mockSessions[0], id: "s-live", name: "Underway", status: "running" },
    ];
    render(SlugPage, { data: { ...baseData, sessions: mixed } });
    const cards = screen.getAllByTestId("live-session");
    expect(cards[0].textContent).toContain("Underway");
    expect(cards[1].textContent).toContain("Waiting Room");
  });

  it("shows player counts and the winner on finished session rows", async () => {
    const history: Session[] = [
      {
        ...mockSessions[0],
        id: "s-done",
        name: "Last Night",
        status: "finished",
        join_code: "DONE01",
        player_count: 4,
        best_player: "Ada Lovelace",
        best_score: 340,
      },
    ];
    render(SlugPage, { data: { ...baseData, sessions: history } });
    await fireEvent.click(screen.getByRole("tab", { name: /Sessions/ }));
    const panel = screen.getByRole("tabpanel");
    expect(within(panel).getByText("4")).not.toBeNull();
    expect(within(panel).getByText("Winner")).not.toBeNull();
    expect(within(panel).getByText("Ada Lovelace")).not.toBeNull();
    expect(within(panel).getByText("340 pts")).not.toBeNull();
  });

  it("keeps the winner column off rows that have no result yet", async () => {
    render(SlugPage, { data: { ...baseData, sessions: mockSessions } });
    await fireEvent.click(screen.getByRole("tab", { name: /Sessions/ }));
    const panel = screen.getByRole("tabpanel");
    // A running session has no final score, so no winner is claimed.
    expect(within(panel).queryByText("Winner")).toBeNull();
    // …but its player count still renders, defaulting to zero.
    expect(within(panel).getByText("Players")).not.toBeNull();
  });

  it("shows a negative winning score, which judges can produce", async () => {
    const docked: Session[] = [
      {
        ...mockSessions[0],
        id: "s-docked",
        status: "finished",
        join_code: "DOCK01",
        player_count: 1,
        best_player: "Anod",
        best_score: -485,
      },
    ];
    render(SlugPage, { data: { ...baseData, sessions: docked } });
    await fireEvent.click(screen.getByRole("tab", { name: /Sessions/ }));
    const panel = screen.getByRole("tabpanel");
    expect(within(panel).getByText("-485 pts")).not.toBeNull();
  });

  it("renders description when present", () => {
    render(SlugPage, { data: baseData });
    expect(screen.getByText("A test project")).not.toBeNull();
  });

  it("edit button hidden for non-owner non-admin", () => {
    render(SlugPage, { data: { ...baseData, currentUserId: "other-user", isAuthenticated: true } });
    expect(document.querySelector('[data-testid="edit-project-btn"]')).toBeNull();
  });

  it("edit button visible for project owner", () => {
    render(SlugPage, { data: { ...baseData, currentUserId: "user-1", isAuthenticated: true } });
    expect(document.querySelector('[data-testid="edit-project-btn"]')).not.toBeNull();
  });

  it("edit button visible for admin", () => {
    render(SlugPage, {
      data: { ...baseData, currentUserId: "other-user", isAuthenticated: true, isAdmin: true },
    });
    expect(document.querySelector('[data-testid="edit-project-btn"]')).not.toBeNull();
  });
  it("keeps a campaign part on the ordinary project page, with only the campaign strip", () => {
    // Opening a part must not turn its page into the campaign's card grid:
    // a part has its own tasks, sessions and board, plus one strip saying
    // where it sits and how to step either way.
    const partProject = {
      ...mockProject,
      id: "part-2",
      name: "Data That Stays",
      parent_project_id: "campaign-1",
      parent_project_slug: "handmade-postgresql",
      parent_project_name: "Handmade PostgreSQL",
      part_ordinal: 1,
      part_count: 0,
    };
    const siblings: ProjectPart[] = [
      { ...partProject, id: "part-1", name: "Speak SQL", part_ordinal: 0, state: "completed" },
      { ...partProject, state: "available" },
    ] as ProjectPart[];

    render(SlugPage, {
      data: { ...baseData, project: partProject, parts: siblings },
    });

    expect(document.querySelectorAll('[data-testid="project-part-state"]')).toHaveLength(0);
    expect(screen.queryByRole("group", { name: "Part layout" })).toBeNull();
    expect(screen.queryByRole("tab", { name: /Parts/ })).toBeNull();

    const strip = screen.getByRole("navigation", { name: "Campaign navigation" });
    expect(within(strip).getByText("Part 2 of 2")).not.toBeNull();
    expect(within(strip).getByText("Speak SQL")).not.toBeNull();
  });
  it("drops the campaign's grid when navigating from it to one of its parts", async () => {
    // SvelteKit reuses this page component across /projects/[slug], so the
    // tab a campaign opened on used to survive into the part you clicked and
    // render the campaign's card grid on top of it.
    const campaign = {
      ...mockProject,
      id: "campaign-1",
      name: "Handmade PostgreSQL",
      slug: "handmade-postgresql",
      part_count: 2,
      task_count: 0,
    };
    const partProject = {
      ...mockProject,
      id: "part-1",
      name: "Speak SQL",
      slug: "handmade-postgresql-1-repl",
      parent_project_id: campaign.id,
      parent_project_slug: campaign.slug,
      parent_project_name: campaign.name,
      part_ordinal: 0,
      part_count: 0,
    };
    const parts = [
      { ...partProject, state: "available" },
      { ...partProject, id: "part-2", name: "Data That Stays", part_ordinal: 1, state: "locked" },
    ] as ProjectPart[];

    const { rerender } = render(SlugPage, {
      data: { ...baseData, project: campaign, parts },
    });
    expect(document.querySelectorAll('[data-testid="project-part-state"]').length).toBe(2);

    await rerender({ data: { ...baseData, project: partProject, parts } });
    expect(document.querySelectorAll('[data-testid="project-part-state"]')).toHaveLength(0);
    expect(screen.queryByRole("group", { name: "Part layout" })).toBeNull();
  });

  it("offers nothing to press on a locked part", () => {
    const partProject = {
      ...mockProject,
      id: "part-2",
      name: "Data That Stays",
      parent_project_id: "campaign-1",
      parent_project_slug: "handmade-postgresql",
      parent_project_name: "Handmade PostgreSQL",
      part_ordinal: 1,
      part_count: 0,
    };
    const parts = [
      { ...partProject, id: "part-1", name: "Speak SQL", part_ordinal: 0, state: "available" },
      { ...partProject, state: "locked" },
    ] as ProjectPart[];

    render(SlugPage, {
      data: {
        ...baseData,
        project: partProject,
        parts,
        currentUserId: "someone",
        isAuthenticated: true,
      },
    });

    expect(screen.queryByRole("button", { name: /Start session/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /Locked/ })).toBeNull();
    expect(screen.getByTestId("project-locked-note").textContent).toMatch(/Finish “Speak SQL”/);
  });
  it("forgets the previous project's sessions when navigating to another one", async () => {
    // Same reuse trap as the tab: the session list was seeded once at mount,
    // so walking from one project to the next kept listing the sessions of
    // the project you came from.
    const other = { ...mockProject, id: "other-1", name: "Other Project", slug: "other-project" };

    const { rerender } = render(SlugPage, {
      data: { ...baseData, sessions: mockSessions },
    });
    await fireEvent.click(screen.getByRole("tab", { name: /Sessions/ }));
    expect(screen.getAllByText("First Session").length).toBeGreaterThan(0);

    await rerender({ data: { ...baseData, project: other, sessions: [] } });
    await fireEvent.click(screen.getByRole("tab", { name: /Sessions/ }));
    expect(screen.queryByText("First Session")).toBeNull();
    expect(screen.getByText("No sessions yet.")).not.toBeNull();
  });
});
