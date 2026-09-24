import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/svelte";
import Page from "./+page.svelte";
import type { PersonalProjectOptions } from "$lib/api";

vi.mock("$app/forms", () => ({ enhance: () => ({ destroy() {} }) }));

const options: PersonalProjectOptions = {
  creation_allowed: true,
  private_allowed: true,
  judges: [
    {
      slug: "correctness",
      name: "Correctness",
      description: "Does what the task asked",
      criteria: ["product"],
      avatar_url: null,
      default: true,
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

function data({
  isPublic = true,
  privateAllowed = true,
  editable = false,
}: { isPublic?: boolean; privateAllowed?: boolean; editable?: boolean } = {}) {
  return {
    options: { ...options, private_allowed: privateAllowed },
    detail: {
      project: {
        id: "p1",
        name: "CSV export",
        slug: "csv-export",
        kind: "personal",
        description: "Add a CSV export.",
        public: isPublic,
        archived_at: null,
        owner_user_id: "me",
        tags: [],
        category: null,
        task_count: 1,
        session_duration_secs: 7200,
        cover_image_url: null,
      },
      spec: {
        version: 1,
        name: "CSV export",
        description: "Add a CSV export.",
        tasks: [],
        judges: ["correctness"],
        session_duration_secs: 7200,
      },
      session_count: editable ? 0 : 2,
      editable,
    },
  };
}

describe("routes/projects/[id]/personal/+page.svelte", () => {
  it("still moves who sees a played project", () => {
    render(Page, { data: data() as never, form: null });
    expect(screen.getByTestId("pp-frozen")).not.toBeNull();
    expect(screen.getByTestId("pp-visibility-now").textContent).toContain("Public");
    expect(screen.getByTestId("pp-visibility-toggle").textContent).toContain("Make it private");
    const field = document.querySelector('input[name="public"]') as HTMLInputElement;
    expect(field.value).toBe("false");
  });

  it("points a free account at Premium instead of a switch it cannot use", () => {
    render(Page, { data: data({ privateAllowed: false }) as never, form: null });
    expect(screen.queryByTestId("pp-visibility-toggle")).toBeNull();
    expect(screen.getByTestId("pp-private-upsell").getAttribute("href")).toBe("/pricing");
  });

  it("always lets a private project go public", () => {
    render(Page, {
      data: data({ isPublic: false, privateAllowed: false }) as never,
      form: null,
    });
    expect(screen.getByTestId("pp-visibility-toggle").textContent).toContain("Make it public");
  });

  it("carries who sees it into the editor before the first session", () => {
    render(Page, { data: data({ isPublic: false, editable: true }) as never, form: null });
    expect((screen.getByTestId("pp-private") as HTMLInputElement).checked).toBe(true);
  });
});
