import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import ProjectFromDescription from "./ProjectFromDescription.svelte";
import type { PersonalProjectOptions } from "$lib/api";
import { ApiError } from "$lib/api/errors";
import { readProjectDraft } from "$lib/project-draft";

const goto = vi.fn();
vi.mock("$app/navigation", () => ({ goto: (...args: unknown[]) => goto(...args) }));

const getOptions = vi.fn();
const suggest = vi.fn();
const create = vi.fn();
vi.mock("$lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("$lib/api")>();
  return {
    ...actual,
    getPersonalProjectOptions: (...args: unknown[]) => getOptions(...args),
    suggestPersonalProject: (...args: unknown[]) => suggest(...args),
    createPersonalProject: (...args: unknown[]) => create(...args),
  };
});

function options(overrides: Partial<PersonalProjectOptions> = {}): PersonalProjectOptions {
  return {
    creation_allowed: true,
    private_allowed: false,
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
      max_tasks: 10,
      max_judges: 6,
      max_name_chars: 120,
      max_description_chars: 8000,
      max_task_title_chars: 200,
      max_task_description_chars: 4000,
    },
    session: { min_secs: 1800, max_secs: 28800, default_secs: 3600 },
    suggest_available: true,
    task_points: 100,
    ...overrides,
  };
}

const DESCRIPTION = "Add a CSV export to the reports page.\nIt honours the table's filters.";
const SUGGESTED = {
  name: "CSV export for reports",
  tasks: [
    { title: "Add the export endpoint", description: "GET /reports.csv returns the rows." },
    { title: "Add the download button", description: "" },
  ],
};

async function describeWork(text = DESCRIPTION) {
  await fireEvent.input(screen.getByTestId("own-project-description"), {
    target: { value: text },
  });
  await fireEvent.click(screen.getByTestId("own-project-suggest"));
}

beforeEach(() => {
  goto.mockReset();
  getOptions.mockReset().mockResolvedValue(options());
  suggest.mockReset().mockResolvedValue(SUGGESTED);
  create.mockReset();
  localStorage.clear();
  sessionStorage.clear();
});

describe("ProjectFromDescription", () => {
  it("sends a visitor who is not signed in to sign up, keeping the description", async () => {
    const onSignIn = vi.fn();
    render(ProjectFromDescription, { isAuthenticated: false, onSignIn });
    await describeWork();
    expect(onSignIn).toHaveBeenCalledOnce();
    expect(suggest).not.toHaveBeenCalled();
    expect(localStorage.getItem("ololo.ownProject.description")).toBe(DESCRIPTION);
  });

  it("drafts the project and shows it: name, tasks, judges and session length", async () => {
    render(ProjectFromDescription, { isAuthenticated: true, onSignIn: vi.fn() });
    await describeWork();
    await waitFor(() => screen.getByTestId("own-project-suggestion"));
    expect(suggest).toHaveBeenCalledWith(DESCRIPTION, undefined);
    expect(screen.getByTestId("own-project-name").textContent?.trim()).toBe(
      "CSV export for reports",
    );
    const tasks = screen.getByTestId("own-project-tasks").textContent ?? "";
    expect(tasks).toContain("Add the export endpoint");
    expect(tasks).toContain("GET /reports.csv returns the rows.");
    expect(tasks).toContain("Add the download button");
    const summary = screen.getByTestId("own-project-summary").textContent ?? "";
    expect(summary).toContain("Reviewed by Correctness, Code Quality");
    expect(summary).toContain("1 h sessions");
    expect(summary).toContain("public");
  });

  it("starts the project: creates it and shows the command that starts a session", async () => {
    create.mockResolvedValue({
      id: "p-1",
      slug: "csv-export-for-reports",
      name: "CSV export for reports",
    });
    render(ProjectFromDescription, { isAuthenticated: true, onSignIn: vi.fn() });
    await describeWork();
    await waitFor(() => screen.getByTestId("own-project-start"));
    await fireEvent.click(screen.getByTestId("own-project-start"));
    await waitFor(() => screen.getByTestId("own-project-created"));
    expect(create).toHaveBeenCalledWith({
      name: "CSV export for reports",
      description: DESCRIPTION,
      tasks: [
        { title: "Add the export endpoint", description: "GET /reports.csv returns the rows." },
        { title: "Add the download button" },
      ],
      judges: ["correctness", "code-quality"],
      session_duration_secs: 3600,
      public: true,
      repo_url: "",
      repo_ref: "",
    });
    expect(screen.getByTestId("own-project-created").textContent).toContain(
      "ololo start csv-export-for-reports",
    );
    // The work is a project now: nothing left to draft twice.
    expect((screen.getByTestId("own-project-description") as HTMLTextAreaElement).value).toBe("");
    expect(localStorage.getItem("ololo.ownProject.description")).toBeNull();
  });

  it("edits the project: hands the draft to /projects/new", async () => {
    render(ProjectFromDescription, { isAuthenticated: true, onSignIn: vi.fn() });
    await describeWork();
    await waitFor(() => screen.getByTestId("own-project-edit"));
    await fireEvent.click(screen.getByTestId("own-project-edit"));
    expect(goto).toHaveBeenCalledWith("/projects/new?draft=1");
    expect(readProjectDraft()).toMatchObject({
      name: "CSV export for reports",
      description: DESCRIPTION,
      tasks: SUGGESTED.tasks,
      judges: ["correctness", "code-quality"],
      session_duration_secs: 3600,
      public: true,
    });
    expect(create).not.toHaveBeenCalled();
  });

  it("drafts on its own once the visitor is back signed in", async () => {
    localStorage.setItem("ololo.ownProject.description", DESCRIPTION);
    localStorage.setItem("ololo.ownProject.pending", String(Date.now()));
    render(ProjectFromDescription, { isAuthenticated: true, onSignIn: vi.fn() });
    await waitFor(() => screen.getByTestId("own-project-suggestion"));
    expect(suggest).toHaveBeenCalledWith(DESCRIPTION, undefined);
    expect(localStorage.getItem("ololo.ownProject.pending")).toBeNull();
  });

  it("offers the project as one task when this server drafts none", async () => {
    getOptions.mockResolvedValue(options({ suggest_available: false }));
    render(ProjectFromDescription, { isAuthenticated: true, onSignIn: vi.fn() });
    await describeWork();
    await waitFor(() => screen.getByTestId("own-project-suggestion"));
    expect(suggest).not.toHaveBeenCalled();
    // The name the server would give it: the first line.
    expect(screen.getByTestId("own-project-name").textContent?.trim()).toBe(
      "Add a CSV export to the reports page.",
    );
    expect(screen.queryByTestId("own-project-tasks")).toBeNull();
    expect(screen.getByTestId("own-project-note").textContent).toContain("one task");
  });

  it("says so when drafts come too fast, without a popup", async () => {
    suggest.mockRejectedValue(new ApiError(429, "rate_limited", {}));
    render(ProjectFromDescription, { isAuthenticated: true, onSignIn: vi.fn() });
    await describeWork();
    await waitFor(() => screen.getByTestId("own-project-error"));
    expect(screen.getByTestId("own-project-error").textContent).toContain("Too many drafts");
    expect(screen.queryByTestId("own-project-suggestion")).toBeNull();
  });
});
