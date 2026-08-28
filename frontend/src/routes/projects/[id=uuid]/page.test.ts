import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import Page from "./+page.svelte";

/**
 * When project.slug is set the server redirects to /projects/{slug},
 * so this page only renders for projects without a slug.
 * UI coverage for the shared layout lives in:
 *   routes/projects/[slug]/page.test.ts
 *
 * These tests exercise the no-slug fallback rendering path.
 */

const PROJECT_ID = "11111111-1111-1111-1111-111111111111";

const mockProject = {
  id: PROJECT_ID,
  name: "No-Slug Project",
  owner_user_id: "22222222-2222-2222-2222-222222222222",
  public: false,
  archived_at: null,
  created_at: "2024-01-01T00:00:00Z",
  updated_at: "2024-01-01T00:00:00Z",
  description: "A project without a slug",
  slug: null,
  has_active_sessions: false,
  category: null,
  tags: [],
  cover_image_url: null,
  task_count: 0,
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

const baseData = {
  project: mockProject,
  sessions: [],
  judges: [],
  taskPreview: [],
  parts: [],
  topPlayers: { players: [], season_players: [], season_start: "2026-08-01T00:00:00Z" },
  message: null,
  isAuthenticated: true,
  isAdmin: false,
  allowProjectCreation: false,
  replayEnabled: false,
  user: null,
  currentUserId: null,
};

describe("routes/projects/[id=uuid]/+page.svelte (no-slug fallback)", () => {
  it("renders project name", () => {
    render(Page, { data: baseData });
    const headings = document.querySelectorAll("h2");
    const hero = Array.from(headings).find((el) => el.textContent?.includes("No-Slug Project"));
    expect(hero).not.toBeNull();
  });

  it("renders empty sessions state", async () => {
    render(Page, { data: baseData });
    // Sessions live behind the Sessions tab (Top Players is the default).
    await fireEvent.click(screen.getByRole("tab", { name: /Sessions/ }));
    expect(screen.getByText("No sessions yet.")).not.toBeNull();
  });

  it("shows edit button for project owner on unarchived project", () => {
    render(Page, {
      data: { ...baseData, currentUserId: "22222222-2222-2222-2222-222222222222" },
    });
    expect(document.querySelector('[data-testid="edit-project-btn"]')).not.toBeNull();
  });

  it("hides edit button for non-owner non-admin", () => {
    render(Page, {
      data: { ...baseData, currentUserId: "33333333-3333-3333-3333-333333333333" },
    });
    expect(document.querySelector('[data-testid="edit-project-btn"]')).toBeNull();
  });

  it("does not render slug field when slug is null", () => {
    render(Page, { data: baseData });
    expect(screen.queryByText(/^Slug$/i)).toBeNull();
  });

  it("does not render Start session button when slug is null", () => {
    render(Page, { data: baseData });
    expect(screen.queryByText("Start session")).toBeNull();
  });
});
