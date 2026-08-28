import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/svelte";
import ProjectCard from "./ProjectCard.svelte";
import type { Project } from "$lib/api";

function project(over: Partial<Project> = {}): Project {
  return {
    id: "p1",
    name: "Handmade PostgreSQL",
    slug: "handmade-postgresql",
    description: "",
    public: true,
    archived_at: null,
    owner_user_id: "owner",
    tags: [],
    category: "Reinvent the Wheel",
    task_count: 0,
    session_duration_secs: 900,
    cover_image_url: null,
    ...over,
  } as Project;
}

describe("ProjectCard", () => {
  it("Counts a campaign's parts instead of the tasks it does not have", () => {
    // A parent holds no tasks of its own, and "0 tasks" read as an empty
    // project rather than as a five-part arc.
    const { container } = render(ProjectCard, { project: project({ part_count: 5 }) });

    expect(container.textContent).not.toContain("0 tasks");
    expect(screen.getByTestId("project-campaign-parts").textContent).toContain("5 parts");
  });

  it("Still counts tasks on an ordinary project", () => {
    const { container } = render(ProjectCard, { project: project({ task_count: 8 }) });
    expect(container.textContent).toContain("8 tasks");
  });

  it("Leaves no stray separator when a segment is missing", () => {
    const { container } = render(ProjectCard, {
      project: project({ part_count: 3, category: null }),
    });
    const meta = container.querySelector(".mb-\\[8px\\]")?.textContent ?? "";
    expect(meta.trim().startsWith("·")).toBe(false);
  });
});
