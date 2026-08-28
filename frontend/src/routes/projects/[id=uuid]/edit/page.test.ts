import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { tick } from "svelte";
import Page from "./+page.svelte";
import ProjectPointsFields from "$lib/components/ProjectPointsFields.svelte";
import ProjectIntervalsFields from "$lib/components/ProjectIntervalsFields.svelte";
import EditAiForms from "./EditAiForms.svelte";

const PROJECT_ID = "11111111-1111-1111-1111-111111111111";

const mockProject = {
  id: PROJECT_ID,
  name: "Test Project",
  owner_user_id: "22222222-2222-2222-2222-222222222222",
  public: false,
  archived_at: null,
  created_at: "2024-01-01T00:00:00Z",
  updated_at: "2024-01-01T00:00:00Z",
  description: "A test project description",
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

const mockTask = {
  id: "33333333-3333-3333-3333-333333333333",
  project_id: PROJECT_ID,
  ordinal: 1,
  title: "My Task",
  description: "task desc",
  tags: [],
  test_template: { kind: "shell" as const, command_template: "echo hi" },
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
};

describe("routes/projects/[id=uuid]/edit/+page.svelte", () => {
  it("Renders project name input", () => {
    render(Page, {
      data: {
        isAuthenticated: true,
        isAdmin: false,
        allowProjectCreation: false,
        replayEnabled: false,
        user: null,
        project: mockProject,
        tasks: [],
        categories: [],
        judges: [],
        taskJudgesMap: {},
      },
      form: null,
    });
    expect(screen.getByTestId("project-name")).not.toBeNull();
  });

  it("Slug input is visible for admins, hidden for non-admins", () => {
    // Non-admin: no slug field
    const { unmount } = render(Page, {
      data: {
        isAuthenticated: true,
        isAdmin: false,
        allowProjectCreation: false,
        replayEnabled: false,
        user: null,
        project: mockProject,
        tasks: [],
        categories: [],
        judges: [],
        taskJudgesMap: {},
      },
      form: null,
    });
    expect(document.querySelector('[data-testid="project-slug"]')).toBeNull();
    unmount();

    // Admin: slug field is present
    render(Page, {
      data: {
        isAuthenticated: true,
        isAdmin: true,
        allowProjectCreation: false,
        replayEnabled: false,
        user: null,
        project: mockProject,
        tasks: [],
        categories: [],
        judges: [],
        taskJudgesMap: {},
      },
      form: null,
    });
    expect(screen.getByTestId("project-slug")).not.toBeNull();
  });

  it("Offers the memory schema to non-admin owners, prefilled from the project", async () => {
    render(Page, {
      data: {
        isAuthenticated: true,
        isAdmin: false,
        allowProjectCreation: false,
        replayEnabled: false,
        user: null,
        project: { ...mockProject, memory_schema: { dev: "npm run dev", port: 1234 } },
        tasks: [],
        categories: [],
        judges: [],
        taskJudgesMap: {},
      },
      form: null,
    });
    const hidden = document.querySelector('input[name="memory_schema_json"]') as HTMLInputElement;
    expect(JSON.parse(hidden.value)).toEqual([
      { key: "dev", value: "npm run dev" },
      { key: "port", value: "1234" },
    ]);
    // The rows themselves live behind the collapsed header.
    await fireEvent.click(screen.getByText(/Session memory/));
    await tick();
    expect(document.querySelectorAll('[data-testid="memory-schema-row"]').length).toBe(2);
  });

  it("Serializes an empty memory schema so saving clears it", () => {
    render(Page, {
      data: {
        isAuthenticated: true,
        isAdmin: false,
        allowProjectCreation: false,
        replayEnabled: false,
        user: null,
        project: mockProject,
        tasks: [],
        categories: [],
        judges: [],
        taskJudgesMap: {},
      },
      form: null,
    });
    const hidden = document.querySelector('input[name="memory_schema_json"]') as HTMLInputElement;
    expect(JSON.parse(hidden.value)).toEqual([]);
  });

  it("Renders save project button", () => {
    render(Page, {
      data: {
        isAuthenticated: true,
        isAdmin: false,
        allowProjectCreation: false,
        replayEnabled: false,
        user: null,
        project: mockProject,
        tasks: [],
        categories: [],
        judges: [],
        taskJudgesMap: {},
      },
      form: null,
    });
    expect(screen.getByTestId("save-project")).not.toBeNull();
  });

  it("Each task has an Edit button", () => {
    render(Page, {
      data: {
        isAuthenticated: true,
        isAdmin: false,
        allowProjectCreation: false,
        replayEnabled: false,
        user: null,
        project: mockProject,
        tasks: [mockTask],
        categories: [],
        judges: [],
        taskJudgesMap: {},
      },
      form: null,
    });
    expect(screen.getByTestId("edit-task-btn-33333333-3333-3333-3333-333333333333")).not.toBeNull();
  });

  it("Task editing controls are always visible regardless of active sessions", () => {
    render(Page, {
      data: {
        isAuthenticated: true,
        isAdmin: false,
        allowProjectCreation: false,
        replayEnabled: false,
        user: null,
        project: { ...mockProject, has_active_sessions: true },
        tasks: [mockTask],
        categories: [],
        judges: [],
        taskJudgesMap: {},
      },
      form: null,
    });
    expect(screen.getByTestId("edit-task-btn-33333333-3333-3333-3333-333333333333")).not.toBeNull();
    expect(screen.queryByTestId("frozen-banner")).toBeNull();
  });

  it("Clicking Edit button opens task form with title input", async () => {
    render(Page, {
      data: {
        isAuthenticated: true,
        isAdmin: false,
        allowProjectCreation: false,
        replayEnabled: false,
        user: null,
        project: mockProject,
        tasks: [mockTask],
        categories: [],
        judges: [],
        taskJudgesMap: {},
      },
      form: null,
    });
    const editBtn = screen.getByTestId("edit-task-btn-33333333-3333-3333-3333-333333333333");
    await fireEvent.click(editBtn);
    const titleInput = document.querySelector('[data-testid="task-title"]');
    expect(titleInput).not.toBeNull();
  });

  it("Clicking Add Task button opens task form", async () => {
    render(Page, {
      data: {
        isAuthenticated: true,
        isAdmin: false,
        allowProjectCreation: false,
        replayEnabled: false,
        user: null,
        project: mockProject,
        tasks: [mockTask],
        categories: [],
        judges: [],
        taskJudgesMap: {},
      },
      form: null,
    });
    const addBtn = screen.getByTestId("add-task-btn");
    await fireEvent.click(addBtn);
    const titleInput = document.querySelector('[data-testid="task-title"]');
    expect(titleInput).not.toBeNull();
  });

  it("Task form title input is present and empty in add mode", async () => {
    render(Page, {
      data: {
        isAuthenticated: true,
        isAdmin: false,
        allowProjectCreation: false,
        replayEnabled: false,
        user: null,
        project: mockProject,
        tasks: [mockTask],
        categories: [],
        judges: [],
        taskJudgesMap: {},
      },
      form: null,
    });
    await fireEvent.click(screen.getByTestId("add-task-btn"));
    const titleInput = document.querySelector(
      '[data-testid="task-title"]',
    ) as HTMLInputElement | null;
    expect(titleInput).not.toBeNull();
    expect(titleInput?.value ?? "").toBe("");
  });

  it("In add mode, command_template markdown field is present and empty", async () => {
    render(Page, {
      data: {
        isAuthenticated: true,
        isAdmin: false,
        allowProjectCreation: false,
        replayEnabled: false,
        user: null,
        project: mockProject,
        tasks: [mockTask],
        categories: [],
        judges: [],
        taskJudgesMap: {},
      },
      form: null,
    });
    await fireEvent.click(screen.getByTestId("add-task-btn"));
    // MarkdownField renders a visually-hidden <textarea> carrying data-testid via ...rest
    const cmdField = document.querySelector(
      'textarea[data-testid="command-template"]',
    ) as HTMLTextAreaElement | null;
    expect(cmdField).not.toBeNull();
    expect(cmdField?.value ?? "").toBe("");
  });

  it("In edit mode, command_template markdown field is pre-populated with task value", async () => {
    render(Page, {
      data: {
        isAuthenticated: true,
        isAdmin: false,
        allowProjectCreation: false,
        replayEnabled: false,
        user: null,
        project: mockProject,
        tasks: [mockTask],
        categories: [],
        judges: [],
        taskJudgesMap: {},
      },
      form: null,
    });
    await fireEvent.click(screen.getByTestId("edit-task-btn-33333333-3333-3333-3333-333333333333"));
    // MarkdownField renders a visually-hidden <textarea> carrying data-testid via ...rest
    const cmdField = document.querySelector(
      'textarea[data-testid="command-template"]',
    ) as HTMLTextAreaElement | null;
    expect(cmdField).not.toBeNull();
    expect(cmdField?.value).toBe("echo hi");
  });

  // ── Tag tests (AC-011, AC-012) ──────────────────────────────────────────

  it("AC-011: Task with tags renders tag badges in the list", () => {
    const taskWithTags = { ...mockTask, tags: ["rust", "backend"] };
    render(Page, {
      data: {
        isAuthenticated: true,
        isAdmin: false,
        allowProjectCreation: false,
        replayEnabled: false,
        user: null,
        project: mockProject,
        tasks: [taskWithTags],
        categories: [],
        judges: [],
        taskJudgesMap: {},
      },
      form: null,
    });
    const container = document.querySelector(
      '[data-testid="task-tags-33333333-3333-3333-3333-333333333333"]',
    );
    expect(container).not.toBeNull();
    expect(container?.textContent).toContain("rust");
    expect(container?.textContent).toContain("backend");
  });

  it("AC-011: Task without tags has no badge container", () => {
    render(Page, {
      data: {
        isAuthenticated: true,
        isAdmin: false,
        allowProjectCreation: false,
        replayEnabled: false,
        user: null,
        project: mockProject,
        tasks: [mockTask], // tags: []
        categories: [],
        judges: [],
        taskJudgesMap: {},
      },
      form: null,
    });
    const container = document.querySelector(
      '[data-testid="task-tags-33333333-3333-3333-3333-333333333333"]',
    );
    expect(container).toBeNull();
  });

  it("AC-012: Add drawer exposes tag input and tags_json hidden field", async () => {
    render(Page, {
      data: {
        isAuthenticated: true,
        isAdmin: false,
        allowProjectCreation: false,
        replayEnabled: false,
        user: null,
        project: mockProject,
        tasks: [],
        categories: [],
        judges: [],
        taskJudgesMap: {},
      },
      form: null,
    });
    await fireEvent.click(screen.getByTestId("add-task-btn"));

    const tagInput = document.querySelector(
      '[data-testid="drawer-tag-input"]',
    ) as HTMLInputElement | null;
    expect(tagInput).not.toBeNull();

    const hiddenInput = document.querySelector(
      '[data-testid="add-task-tags-json"]',
    ) as HTMLInputElement | null;
    expect(hiddenInput).not.toBeNull();
    // Initially empty
    expect(JSON.parse(hiddenInput?.value ?? "[]")).toEqual([]);
  });

  it("AC-012: Typing a tag with comma confirms the chip and updates tags_json", async () => {
    render(Page, {
      data: {
        isAuthenticated: true,
        isAdmin: false,
        allowProjectCreation: false,
        replayEnabled: false,
        user: null,
        project: mockProject,
        tasks: [],
        categories: [],
        judges: [],
        taskJudgesMap: {},
      },
      form: null,
    });
    await fireEvent.click(screen.getByTestId("add-task-btn"));

    const tagInput = document.querySelector('[data-testid="drawer-tag-input"]') as HTMLInputElement;
    expect(tagInput).not.toBeNull();

    // Comma-suffix triggers confirm in the oninput handler (no separate keydown needed).
    await fireEvent.input(tagInput, { target: { value: "mytag," } });
    // Extra tick to let Svelte flush all pending reactive updates in jsdom.
    await tick();

    // The visible chip should appear.
    const tagsContainer = document.querySelector('[data-testid="drawer-task-tags"]');
    expect(tagsContainer).not.toBeNull();
    expect(tagsContainer?.textContent).toContain("mytag");

    // The hidden form field should reflect the tag list.
    const hiddenInput = document.querySelector(
      '[data-testid="add-task-tags-json"]',
    ) as HTMLInputElement;
    expect(hiddenInput).not.toBeNull();
    expect(JSON.parse(hiddenInput?.value ?? "[]")).toEqual(["mytag"]);
  });
});
