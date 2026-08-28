import { beforeEach, describe, expect, it, vi } from "vitest";
import { request } from "./core";
import {
  listProjects,
  createProject,
  getProject,
  patchProject,
  deleteProject,
  getProjectBySlug,
  getPrivateProjectBySlug,
  getProjectCategories,
  listProjectTasks,
  createProjectTask,
  patchProjectTask,
  deleteProjectTask,
  reorderTasks,
  beautifyDescription,
  generateTasks,
  beautifyTaskDescription,
  generateTaskTests,
  beautifyTaskDescriptionById,
  generateTaskTestsById,
  exportProject,
  importProject,
} from "./projects";
import type { CreateProjectRequest, PatchProjectRequest, TaskInput, TaskDraft } from "./types";

vi.mock("./core", () => ({ request: vi.fn() }));

const mockRequest = vi.mocked(request);

describe("projects api", () => {
  beforeEach(() => mockRequest.mockReset());

  it("listProjects defaults to /api/projects", async () => {
    mockRequest.mockResolvedValue({ projects: [{ id: "p1" }] });
    const res = await listProjects();
    expect(mockRequest).toHaveBeenCalledWith("/api/projects", { fetch: undefined });
    expect(res).toEqual([{ id: "p1" }]);
  });

  it("listProjects includes archived flag", async () => {
    mockRequest.mockResolvedValue({ projects: [] });
    await listProjects(true);
    expect(mockRequest).toHaveBeenCalledWith("/api/projects?include_archived=true", {
      fetch: undefined,
    });
  });

  it("createProject POSTs the request", async () => {
    const req: CreateProjectRequest = { name: "P" };
    mockRequest.mockResolvedValue({ id: "p1" });
    await createProject(req);
    expect(mockRequest).toHaveBeenCalledWith("/api/projects", {
      method: "POST",
      body: req,
      fetch: undefined,
    });
  });

  it("getProject GETs by id", async () => {
    mockRequest.mockResolvedValue({ id: "p1" });
    await getProject("p1");
    expect(mockRequest).toHaveBeenCalledWith("/api/projects/p1", { fetch: undefined });
  });

  it("patchProject PATCHes by id", async () => {
    const req: PatchProjectRequest = { name: "P2" };
    mockRequest.mockResolvedValue({ id: "p1" });
    await patchProject("p1", req);
    expect(mockRequest).toHaveBeenCalledWith("/api/projects/p1", {
      method: "PATCH",
      body: req,
      fetch: undefined,
    });
  });

  it("deleteProject DELETEs by id", async () => {
    mockRequest.mockResolvedValue(undefined);
    await deleteProject("p1");
    expect(mockRequest).toHaveBeenCalledWith("/api/projects/p1", {
      method: "DELETE",
      fetch: undefined,
    });
  });

  it("getProjectBySlug GETs by slug", async () => {
    mockRequest.mockResolvedValue({ id: "p1" });
    await getProjectBySlug("my-slug");
    expect(mockRequest).toHaveBeenCalledWith("/api/projects/by-slug/my-slug", {
      fetch: undefined,
    });
  });

  it("getPrivateProjectBySlug GETs by user and slug", async () => {
    mockRequest.mockResolvedValue({ id: "p1" });
    await getPrivateProjectBySlug("u1", "my-slug");
    expect(mockRequest).toHaveBeenCalledWith("/api/projects/u/u1/by-slug/my-slug", {
      fetch: undefined,
    });
  });

  it("getProjectCategories unwraps .categories", async () => {
    mockRequest.mockResolvedValue({ categories: ["a", "b"] });
    const res = await getProjectCategories();
    expect(mockRequest).toHaveBeenCalledWith("/api/projects/categories", { fetch: undefined });
    expect(res).toEqual(["a", "b"]);
  });

  it("listProjectTasks unwraps .tasks", async () => {
    mockRequest.mockResolvedValue({ tasks: [{ id: "t1" }] });
    const res = await listProjectTasks("p1");
    expect(mockRequest).toHaveBeenCalledWith("/api/projects/p1/tasks", { fetch: undefined });
    expect(res).toEqual([{ id: "t1" }]);
  });

  it("createProjectTask POSTs a task", async () => {
    const task: TaskInput = {
      title: "T",
      test_template: { kind: "shell", command_template: "echo ok" },
    };
    mockRequest.mockResolvedValue({ id: "t1" });
    await createProjectTask("p1", task);
    expect(mockRequest).toHaveBeenCalledWith("/api/projects/p1/tasks", {
      method: "POST",
      body: task,
      fetch: undefined,
    });
  });

  it("patchProjectTask PATCHes a task", async () => {
    const task: TaskInput = {
      title: "T2",
      test_template: { kind: "shell", command_template: "echo ok" },
    };
    mockRequest.mockResolvedValue({ id: "t1" });
    await patchProjectTask("p1", "t1", task);
    expect(mockRequest).toHaveBeenCalledWith("/api/projects/p1/tasks/t1", {
      method: "PATCH",
      body: task,
      fetch: undefined,
    });
  });

  it("deleteProjectTask DELETEs a task", async () => {
    mockRequest.mockResolvedValue(undefined);
    await deleteProjectTask("p1", "t1");
    expect(mockRequest).toHaveBeenCalledWith("/api/projects/p1/tasks/t1", {
      method: "DELETE",
      fetch: undefined,
    });
  });

  it("reorderTasks PATCHes with task_ids", async () => {
    mockRequest.mockResolvedValue(undefined);
    await reorderTasks("p1", ["t2", "t1"]);
    expect(mockRequest).toHaveBeenCalledWith("/api/projects/p1/tasks/reorder", {
      method: "PATCH",
      body: { task_ids: ["t2", "t1"] },
      fetch: undefined,
    });
  });

  it("beautifyDescription POSTs description", async () => {
    mockRequest.mockResolvedValue({ description: "nice" });
    const res = await beautifyDescription("p1", "raw");
    expect(mockRequest).toHaveBeenCalledWith("/api/projects/p1/beautify-description", {
      method: "POST",
      body: { description: "raw" },
      fetch: undefined,
    });
    expect(res).toEqual({ description: "nice" });
  });

  it("generateTasks POSTs description and unwraps .tasks", async () => {
    const drafts: TaskDraft[] = [{ title: "T", description: "d" }];
    mockRequest.mockResolvedValue({ tasks: drafts });
    const res = await generateTasks("p1", "idea");
    expect(mockRequest).toHaveBeenCalledWith("/api/projects/p1/generate-tasks", {
      method: "POST",
      body: { description: "idea" },
      fetch: undefined,
    });
    expect(res).toEqual({ tasks: drafts });
  });

  it("beautifyTaskDescription POSTs to tasks/ai/beautify", async () => {
    mockRequest.mockResolvedValue({ description: "nice" });
    await beautifyTaskDescription("p1", "raw");
    expect(mockRequest).toHaveBeenCalledWith("/api/projects/p1/tasks/ai/beautify", {
      method: "POST",
      body: { description: "raw" },
      fetch: undefined,
    });
  });

  it("generateTaskTests POSTs title and description", async () => {
    mockRequest.mockResolvedValue({ command_template: "c", answer_template: "a", fixtures: [] });
    await generateTaskTests("p1", "T", "D");
    expect(mockRequest).toHaveBeenCalledWith("/api/projects/p1/tasks/ai/generate", {
      method: "POST",
      body: { title: "T", description: "D" },
      fetch: undefined,
    });
  });

  it("beautifyTaskDescriptionById POSTs to per-task beautify", async () => {
    mockRequest.mockResolvedValue({ description: "nice" });
    await beautifyTaskDescriptionById("p1", "t1", "raw");
    expect(mockRequest).toHaveBeenCalledWith("/api/projects/p1/tasks/t1/ai/beautify", {
      method: "POST",
      body: { description: "raw" },
      fetch: undefined,
    });
  });

  it("generateTaskTestsById POSTs to per-task generate", async () => {
    mockRequest.mockResolvedValue({ command_template: "c", answer_template: "a", fixtures: [] });
    await generateTaskTestsById("p1", "t1", "T", "D");
    expect(mockRequest).toHaveBeenCalledWith("/api/projects/p1/tasks/t1/ai/generate", {
      method: "POST",
      body: { title: "T", description: "D" },
      fetch: undefined,
    });
  });

  it("exportProject GETs the export envelope", async () => {
    mockRequest.mockResolvedValue({ schema_version: 1, project: {}, tasks: [] });
    await exportProject("p1");
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/projects/p1/export", {
      fetch: undefined,
    });
  });

  it("importProject reads the file and POSTs parsed JSON", async () => {
    const parsed = { schema_version: 1, project: {}, tasks: [] };
    const file = {
      text: async () => JSON.stringify(parsed),
    } as unknown as File;
    mockRequest.mockResolvedValue({ project_id: "p1", name: "P" });
    const res = await importProject(file);
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/projects/import", {
      method: "POST",
      body: parsed,
      fetch: undefined,
    });
    expect(res).toEqual({ project_id: "p1", name: "P" });
  });
});
