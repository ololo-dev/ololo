import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { tick } from "svelte";
import PersonalProjectForm from "./PersonalProjectForm.svelte";
import type { PersonalProjectOptions } from "$lib/api";

vi.mock("$app/forms", () => ({ enhance: () => ({ destroy() {} }) }));

const suggest = vi.fn();
vi.mock("$lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/api")>();
  return {
    ...actual,
    suggestPersonalTasks: (...args: unknown[]) => suggest(...args),
  };
});

function options(overrides: Partial<PersonalProjectOptions> = {}): PersonalProjectOptions {
  return {
    creation_allowed: true,
    judges: [
      {
        slug: "correctness",
        name: "Correctness",
        description: "d",
        criteria: ["product"],
        avatar_url: null,
        default: true,
      },
      {
        slug: "code-quality",
        name: "Code Quality",
        description: "d",
        criteria: ["cleanliness"],
        avatar_url: null,
        default: true,
      },
      {
        slug: "architecture",
        name: "Architecture",
        description: "d",
        criteria: ["architecture"],
        avatar_url: null,
        default: false,
      },
    ],
    limits: {
      max_tasks: 2,
      max_judges: 2,
      max_name_chars: 120,
      max_description_chars: 8000,
      max_task_title_chars: 200,
      max_task_description_chars: 4000,
    },
    session: { min_secs: 1800, max_secs: 28800, default_secs: 7200 },
    suggest_available: true,
    task_points: 100,
    ...overrides,
  };
}

function hidden(name: string): string {
  const el = document.querySelector(`input[name="${name}"]`) as HTMLInputElement | null;
  return el?.value ?? "";
}

describe("PersonalProjectForm", () => {
  beforeEach(() => suggest.mockReset());

  it("starts as one task with the default panel", () => {
    render(PersonalProjectForm, {
      options: options(),
      submitLabel: "Create project",
      cancelHref: "/projects",
    });
    expect(screen.getByTestId("pp-single-task")).not.toBeNull();
    expect(JSON.parse(hidden("judges_json"))).toEqual(["correctness", "code-quality"]);
    expect(hidden("session_duration_secs")).toBe("7200");
    expect(screen.getByTestId("pp-summary").textContent).toContain(
      "about 3 judge reviews per session",
    );
    // Nothing to submit without a description.
    expect((screen.getByTestId("pp-submit") as HTMLButtonElement).disabled).toBe(true);
  });

  it("builds the map task by task, up to the limit", async () => {
    render(PersonalProjectForm, {
      options: options(),
      submitLabel: "Create project",
      cancelHref: "/projects",
    });
    const add = screen.getByTestId("pp-add-task") as HTMLButtonElement;
    await fireEvent.click(add);
    await fireEvent.click(add);
    expect(add.disabled).toBe(true);
    const titles = screen.getAllByTestId("pp-task-title") as HTMLInputElement[];
    expect(titles).toHaveLength(2);
    await fireEvent.input(titles[0], { target: { value: "Endpoint" } });
    await fireEvent.input(titles[1], { target: { value: "Button" } });
    await tick();
    expect(JSON.parse(hidden("tasks_json"))).toEqual([
      { title: "Endpoint", description: "" },
      { title: "Button", description: "" },
    ]);
    expect(screen.getByTestId("pp-summary").textContent).toContain("2 tasks × 2 judges");

    await fireEvent.click(screen.getAllByTestId("pp-task-remove")[0]);
    expect(JSON.parse(hidden("tasks_json"))).toEqual([{ title: "Button", description: "" }]);
  });

  it("caps the panel and keeps at least the choice explicit", async () => {
    render(PersonalProjectForm, {
      options: options(),
      submitLabel: "Create project",
      cancelHref: "/projects",
    });
    const architecture = screen.getByTestId("pp-judge-architecture") as HTMLInputElement;
    expect(architecture.disabled).toBe(true);
    await fireEvent.change(screen.getByTestId("pp-judge-code-quality"));
    expect(architecture.disabled).toBe(false);
    await fireEvent.change(architecture);
    expect(JSON.parse(hidden("judges_json"))).toEqual(["correctness", "architecture"]);
  });

  it("previews a suggested map before it replaces anything", async () => {
    suggest.mockResolvedValue([
      { title: "Serve the CSV", description: "GET /x.csv" },
      { title: "Add the button" },
    ]);
    render(PersonalProjectForm, {
      options: options(),
      submitLabel: "Create project",
      cancelHref: "/projects",
    });
    await fireEvent.input(screen.getByTestId("pp-description"), {
      target: { value: "Add a CSV export." },
    });
    await fireEvent.click(screen.getByTestId("pp-suggest"));
    await tick();
    await tick();
    expect(suggest).toHaveBeenCalledWith("Add a CSV export.", undefined);
    expect(screen.getByTestId("pp-suggestion").textContent).toContain("Serve the CSV");
    expect(screen.queryAllByTestId("pp-task-title")).toHaveLength(0);
    await fireEvent.click(screen.getByTestId("pp-suggestion-accept"));
    expect(JSON.parse(hidden("tasks_json"))).toEqual([
      { title: "Serve the CSV", description: "GET /x.csv" },
      { title: "Add the button", description: "" },
    ]);
  });

  it("hides the suggestion button when no model is configured", () => {
    render(PersonalProjectForm, {
      options: options({ suggest_available: false }),
      submitLabel: "Create project",
      cancelHref: "/projects",
    });
    expect(screen.queryByTestId("pp-suggest")).toBeNull();
  });
});
