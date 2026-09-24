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
    private_allowed: true,
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
      "About 3 judge reviews per session",
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
    expect(screen.getByTestId("pp-task-count").textContent?.trim()).toBe("2");
    expect(screen.getByTestId("pp-summary").textContent).toContain("About 5 judge reviews");

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

  it("gives a task judges of its own and takes them back", async () => {
    render(PersonalProjectForm, {
      options: options(),
      submitLabel: "Create project",
      cancelHref: "/projects",
    });
    await fireEvent.input(screen.getByTestId("pp-description"), {
      target: { value: "Add a CSV export." },
    });
    const add = screen.getByTestId("pp-add-task");
    await fireEvent.click(add);
    await fireEvent.click(add);
    const titles = screen.getAllByTestId("pp-task-title");
    await fireEvent.input(titles[0], { target: { value: "Endpoint" } });
    await fireEvent.input(titles[1], { target: { value: "Button" } });

    // Its own panel starts as a copy of the default one, under the same cap.
    await fireEvent.click(screen.getAllByTestId("pp-task-customize")[0]);
    expect(screen.getAllByTestId("pp-task-customize")).toHaveLength(1);
    const architecture = screen.getByTestId("pp-task-judge-architecture") as HTMLButtonElement;
    expect(architecture.disabled).toBe(true);
    await fireEvent.click(screen.getByTestId("pp-task-judge-code-quality"));
    await fireEvent.click(screen.getByTestId("pp-task-judge-correctness"));
    await fireEvent.click(architecture);
    expect(architecture.getAttribute("aria-pressed")).toBe("true");
    await tick();
    expect(JSON.parse(hidden("tasks_json"))).toEqual([
      { title: "Endpoint", description: "", judges: ["architecture"] },
      { title: "Button", description: "" },
    ]);
    // The default panel is untouched; the task's own counts in the reviews.
    expect(JSON.parse(hidden("judges_json"))).toEqual(["correctness", "code-quality"]);
    expect(screen.getByTestId("pp-own-panels").textContent?.trim()).toBe("1");
    expect(screen.getByTestId("pp-summary").textContent).toContain("About 4 judge reviews");

    // A task's own panel cannot be empty.
    const submit = screen.getByTestId("pp-submit") as HTMLButtonElement;
    expect(submit.disabled).toBe(false);
    await fireEvent.click(architecture);
    expect(submit.disabled).toBe(true);
    expect(screen.getByTestId("pp-empty-panels").textContent).toContain("Task 1 has no judges");

    await fireEvent.click(screen.getByTestId("pp-task-inherit"));
    expect(submit.disabled).toBe(false);
    expect(JSON.parse(hidden("tasks_json"))).toEqual([
      { title: "Endpoint", description: "" },
      { title: "Button", description: "" },
    ]);
  });

  it("re-renders a task's own judges, minus the ones no longer offered", async () => {
    render(PersonalProjectForm, {
      options: options(),
      initial: {
        name: "CSV export",
        description: "Add a CSV export.",
        tasks: [
          { title: "Endpoint", description: "", judges: ["architecture", "retired"] },
          { title: "Button", description: "" },
        ],
        judges: ["correctness"],
        session_duration_secs: 3600,
      },
      submitLabel: "Save changes",
      cancelHref: "/projects/p",
    });
    expect(screen.getAllByTestId("pp-task-panel")).toHaveLength(1);
    expect(JSON.parse(hidden("tasks_json"))).toEqual([
      { title: "Endpoint", description: "", judges: ["architecture"] },
      { title: "Button", description: "" },
    ]);
    expect(screen.getByTestId("pp-task-judges").textContent).toContain("Correctness");
  });

  it("is public unless kept private, and says what that means", async () => {
    render(PersonalProjectForm, {
      options: options(),
      submitLabel: "Create project",
      cancelHref: "/projects",
    });
    const picked = () =>
      (document.querySelector('input[name="public"]:checked') as HTMLInputElement).value;
    expect(picked()).toBe("true");
    expect(screen.getByTestId("pp-visibility-summary").textContent?.trim()).toBe("Anyone");
    expect(screen.queryByTestId("pp-private-upsell")).toBeNull();

    await fireEvent.click(screen.getByTestId("pp-private"));
    expect(picked()).toBe("false");
    expect(screen.getByTestId("pp-visibility-summary").textContent?.trim()).toBe("Only you");
  });

  it("offers private to Premium only, and keeps a private one private", () => {
    const { unmount } = render(PersonalProjectForm, {
      options: options({ private_allowed: false }),
      submitLabel: "Create project",
      cancelHref: "/projects",
    });
    expect((screen.getByTestId("pp-private") as HTMLInputElement).disabled).toBe(true);
    expect(screen.getByTestId("pp-private-upsell").getAttribute("href")).toBe("/pricing");
    unmount();

    render(PersonalProjectForm, {
      options: options({ private_allowed: false }),
      initial: {
        name: "CSV export",
        description: "Add a CSV export.",
        tasks: [],
        judges: ["correctness"],
        session_duration_secs: 3600,
        public: false,
      },
      submitLabel: "Save changes",
      cancelHref: "/projects/p",
    });
    const kept = screen.getByTestId("pp-private") as HTMLInputElement;
    expect(kept.checked).toBe(true);
    expect(kept.disabled).toBe(false);
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
