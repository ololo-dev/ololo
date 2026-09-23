import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/svelte";
import NewProjectPage from "./+page.svelte";
import type { PersonalProjectOptions } from "$lib/api";

const options: PersonalProjectOptions = {
  creation_allowed: true,
  judges: [
    {
      slug: "correctness",
      name: "Correctness",
      description: "Does what the task asked",
      criteria: ["product"],
      avatar_url: null,
      default: true,
    },
    {
      slug: "architecture",
      name: "Architecture",
      description: "Structure",
      criteria: ["architecture"],
      avatar_url: null,
      default: false,
    },
  ],
  limits: {
    max_tasks: 10,
    max_judges: 6,
    max_name_chars: 120,
    max_description_chars: 8000,
    max_task_title_chars: 200,
    max_task_description_chars: 4000,
  },
  session: { min_secs: 1800, max_secs: 28800, default_secs: 7200 },
  suggest_available: false,
  task_points: 100,
};

const baseData = {
  isAuthenticated: true,
  isAdmin: false,
  allowProjectCreation: true,
  plansEnabled: false,
  replayEnabled: false,
  legalPages: [],
  user: null,
  options,
  initial: null,
};

describe("routes/projects/new/+page.svelte", () => {
  it("renders the personal project form", () => {
    render(NewProjectPage, { data: baseData, form: null });
    expect(screen.getByTestId("personal-project-form")).not.toBeNull();
    expect(screen.getByTestId("pp-description")).not.toBeNull();
    expect(screen.getByTestId("pp-single-task")).not.toBeNull();
  });

  it("offers the challenge editor to admins only", () => {
    render(NewProjectPage, { data: baseData, form: null });
    expect(screen.queryByText(/challenge project for the catalog/)).toBeNull();
    render(NewProjectPage, { data: { ...baseData, isAdmin: true }, form: null });
    expect(screen.getByText(/challenge project for the catalog/)).not.toBeNull();
  });

  it("re-renders the values and the error of a refused request", () => {
    render(NewProjectPage, {
      data: baseData,
      form: {
        error: "invalid_personal_project",
        field: "tasks",
        detail: "a navigation map holds at most 10 tasks",
        values: {
          name: "CSV export",
          description: "Add a CSV export.",
          tasks: [{ title: "Endpoint", description: "" }],
          judges: ["correctness"],
          session_duration_secs: 3600,
        },
      },
    });
    expect(screen.getByTestId("pp-error").textContent).toContain(
      "Navigation map: a navigation map holds at most 10 tasks",
    );
    expect((screen.getByTestId("pp-name") as HTMLInputElement).value).toBe("CSV export");
    expect(screen.getAllByTestId("pp-task-title")).toHaveLength(1);
  });
});
