import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/svelte";
import UserSlugPage from "./+page.svelte";

const mockProject = {
  id: "proj-2",
  name: "Private Project",
  owner_user_id: "user-abc",
  public: false,
  archived_at: null,
  created_at: "2024-01-01T00:00:00Z",
  updated_at: "2024-01-01T00:00:00Z",
  description: "A private project",
  slug: "private-project",
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

const mockTasks = [
  {
    id: "task-2",
    project_id: "proj-2",
    ordinal: 1,
    title: "Private task",
    description: "Secret work",
    tags: [],
    test_template: { kind: "shell" as const, command_template: "npm test" },
    created_at: "2024-01-01T00:00:00Z",
    result_count: 0,
    session_count: 0,
    points: { value: 10, fail: -5, no_response: -10, completion_bonus: 10 },
    intervals: {
      deadline_secs: 60,
      min_interval_secs: 5,
      interval_increment_secs: 5,
      max_interval_secs: 60,
    },
  },
];

describe("routes/projects/u[userId]/[slug]/+page.svelte", () => {
  it("renders project name", () => {
    render(UserSlugPage, {
      data: {
        project: mockProject,
        tasks: mockTasks,
        isAuthenticated: true,
        isAdmin: false,
        allowProjectCreation: false,
        replayEnabled: false,
        user: null,
      },
    });
    expect(screen.getByText("Private Project")).not.toBeNull();
  });

  it("renders back link", () => {
    render(UserSlugPage, {
      data: {
        project: mockProject,
        tasks: mockTasks,
        isAuthenticated: true,
        isAdmin: false,
        allowProjectCreation: false,
        replayEnabled: false,
        user: null,
      },
    });
    const link = document.querySelector('a[href="/projects"]');
    expect(link).not.toBeNull();
  });

  it("renders full view link to uuid route", () => {
    render(UserSlugPage, {
      data: {
        project: mockProject,
        tasks: mockTasks,
        isAuthenticated: true,
        isAdmin: false,
        allowProjectCreation: false,
        replayEnabled: false,
        user: null,
      },
    });
    const link = document.querySelector('a[href="/projects/proj-2"]');
    expect(link).not.toBeNull();
  });

  it("renders task list", () => {
    render(UserSlugPage, {
      data: {
        project: mockProject,
        tasks: mockTasks,
        isAuthenticated: true,
        isAdmin: false,
        allowProjectCreation: false,
        replayEnabled: false,
        user: null,
      },
    });
    expect(screen.getByText(/Private task/)).not.toBeNull();
  });

  it("renders empty state when no tasks", () => {
    render(UserSlugPage, {
      data: {
        project: mockProject,
        tasks: [],
        isAuthenticated: true,
        isAdmin: false,
        allowProjectCreation: false,
        replayEnabled: false,
        user: null,
      },
    });
    expect(screen.getByText("No tasks yet.")).not.toBeNull();
  });

  it("shows private label for private project", () => {
    render(UserSlugPage, {
      data: {
        project: mockProject,
        tasks: [],
        isAuthenticated: true,
        isAdmin: false,
        allowProjectCreation: false,
        replayEnabled: false,
        user: null,
      },
    });
    const el = document.querySelector("p.mt-1");
    expect(el?.textContent).toMatch(/Private/);
  });
});
