import { describe, it, expect, vi } from "vitest";
import {
  ApiError,
  joinSession,
  createSession,
  listSessions,
  registerAgent,
  createProject,
  getProjectBySlug,
  getPrivateProjectBySlug,
  reorderTasks,
  type Project,
} from "./api";

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

describe("lib/api.ts", () => {
  it("listSessions_returns_array_on_200", async () => {
    const sessions = [
      {
        id: "sess1",
        name: "demo",
        status: "lobby",
        owner_id: "00000000-0000-0000-0000-000000000001",
        created_at: "2026-05-02T00:00:00Z",
      },
    ];
    const f = vi.fn().mockResolvedValue(jsonResponse(200, { sessions }));
    const out = await listSessions({ fetch: f as unknown as typeof fetch });
    expect(out).toEqual(sessions);
    expect(f).toHaveBeenCalledWith("/api/sessions", expect.objectContaining({ method: "GET" }));
  });

  it("createSession_throws_on_422", async () => {
    const f = vi.fn().mockResolvedValue(jsonResponse(422, { error: "invalid_name" }));
    await expect(
      createSession("", "", { fetch: f as unknown as typeof fetch }),
    ).rejects.toMatchObject({
      name: "ApiError",
      status: 422,
      code: "invalid_name",
    });
  });

  it("joinSession_handles_404", async () => {
    const f = vi.fn().mockResolvedValue(jsonResponse(404, { error: "not_found" }));
    let caught: unknown;
    try {
      await joinSession("BADCODE", { fetch: f as unknown as typeof fetch });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(ApiError);
    expect((caught as ApiError).status).toBe(404);
    expect((caught as ApiError).code).toBe("not_found");
  });

  it("registerAgent_returns_token_once", async () => {
    const payload = {
      agent: {
        id: "a1",
        session_id: "sess1",
        user_id: "u1",
        kind: "claude-code",
        model: "sonnet",
        display_name: "A",
        registered_at: "2026-05-02T00:00:00Z",
        revoked_at: null,
      },
      observer_token: "obs_secret_token_123",
    };
    const f = vi.fn().mockResolvedValue(jsonResponse(201, payload));
    const result = await registerAgent(
      "sess1",
      { kind: "claude-code", model: "sonnet", display_name: "A" },
      { fetch: f as unknown as typeof fetch },
    );
    expect(result.observer_token).toBe("obs_secret_token_123");
    expect(result.agent.id).toBe("a1");
    expect(f.mock.calls[0][1]).toMatchObject({ method: "POST" });
  });

  // ---- Project type includes new fields ----
  it("Project_type_includes_description_slug_has_active_sessions", () => {
    // Type-level test: ensure the Project interface has the new fields
    const project: Project = {
      id: "p1",
      name: "Test Project",
      owner_user_id: "u1",
      public: true,
      archived_at: null,
      created_at: "2026-05-04T00:00:00Z",
      updated_at: "2026-05-04T00:00:00Z",
      description: "A test project description",
      slug: "test-project",
      has_active_sessions: false,
      category: null,
      tags: [],
      cover_image_url: null,
      task_count: 0,
      points: { value: 10, fail: -5, no_response: -10, completion_bonus: 10 },
      points_range: null,
      intervals: {
        deadline_secs: 60,
        min_interval_secs: 5,
        interval_increment_secs: 5,
        max_interval_secs: 60,
      },
      session_duration_secs: 3600,
      idle_timeout_secs: 300,
      show_tasks: true,
    };
    expect(project.description).toBe("A test project description");
    expect(project.slug).toBe("test-project");
    expect(project.has_active_sessions).toBe(false);
    expect(project.session_duration_secs).toBe(3600);
  });

  // ---- createProject accepts description (slug is admin-only via PATCH) ----
  it("createProject_accepts_description", async () => {
    const projectPayload: Project = {
      id: "p1",
      name: "My Project",
      owner_user_id: "u1",
      public: false,
      archived_at: null,
      created_at: "2026-05-04T00:00:00Z",
      updated_at: "2026-05-04T00:00:00Z",
      description: "desc",
      slug: null,
      has_active_sessions: false,
      category: null,
      tags: [],
      cover_image_url: null,
      task_count: 0,
      points: { value: 10, fail: -5, no_response: -10, completion_bonus: 10 },
      points_range: null,
      intervals: {
        deadline_secs: 60,
        min_interval_secs: 5,
        interval_increment_secs: 5,
        max_interval_secs: 60,
      },
      session_duration_secs: 3600,
      idle_timeout_secs: 300,
      show_tasks: true,
    };
    const f = vi.fn().mockResolvedValue(jsonResponse(201, projectPayload));
    const result = await createProject(
      { name: "My Project", description: "desc", public: false },
      { fetch: f as unknown as typeof fetch },
    );
    expect(result.slug).toBeNull();
    const body = JSON.parse(f.mock.calls[0][1].body);
    expect(body.description).toBe("desc");
  });

  // ---- getProjectBySlug ----
  it("getProjectBySlug_calls_correct_endpoint", async () => {
    const projectPayload: Project = {
      id: "p1",
      name: "My Project",
      owner_user_id: "u1",
      public: true,
      archived_at: null,
      created_at: "2026-05-04T00:00:00Z",
      updated_at: "2026-05-04T00:00:00Z",
      description: "desc",
      slug: "my-slug",
      has_active_sessions: false,
      category: null,
      tags: [],
      cover_image_url: null,
      task_count: 0,
      points: { value: 10, fail: -5, no_response: -10, completion_bonus: 10 },
      points_range: null,
      intervals: {
        deadline_secs: 60,
        min_interval_secs: 5,
        interval_increment_secs: 5,
        max_interval_secs: 60,
      },
      session_duration_secs: 3600,
      idle_timeout_secs: 300,
      show_tasks: true,
    };
    const f = vi.fn().mockResolvedValue(jsonResponse(200, projectPayload));
    const result = await getProjectBySlug("my-slug", { fetch: f as unknown as typeof fetch });
    expect(result.slug).toBe("my-slug");
    expect(f).toHaveBeenCalledWith(
      "/api/projects/by-slug/my-slug",
      expect.objectContaining({ method: "GET" }),
    );
  });

  // ---- getPrivateProjectBySlug ----
  it("getPrivateProjectBySlug_calls_correct_endpoint", async () => {
    const projectPayload: Project = {
      id: "p1",
      name: "My Project",
      owner_user_id: "user-123",
      public: false,
      archived_at: null,
      created_at: "2026-05-04T00:00:00Z",
      updated_at: "2026-05-04T00:00:00Z",
      description: "desc",
      slug: "my-slug",
      has_active_sessions: true,
      category: null,
      tags: [],
      cover_image_url: null,
      task_count: 0,
      points: { value: 10, fail: -5, no_response: -10, completion_bonus: 10 },
      points_range: null,
      intervals: {
        deadline_secs: 60,
        min_interval_secs: 5,
        interval_increment_secs: 5,
        max_interval_secs: 60,
      },
      session_duration_secs: 3600,
      idle_timeout_secs: 300,
      show_tasks: true,
    };
    const f = vi.fn().mockResolvedValue(jsonResponse(200, projectPayload));
    const result = await getPrivateProjectBySlug("user-123", "my-slug", {
      fetch: f as unknown as typeof fetch,
    });
    expect(result.slug).toBe("my-slug");
    expect(f).toHaveBeenCalledWith(
      "/api/projects/u/user-123/by-slug/my-slug",
      expect.objectContaining({ method: "GET" }),
    );
  });

  // ---- reorderTasks ----
  it("reorderTasks_calls_patch_with_task_ids", async () => {
    const f = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
    await reorderTasks("project-id", ["task-1", "task-2"], { fetch: f as unknown as typeof fetch });
    expect(f).toHaveBeenCalledWith(
      "/api/projects/project-id/tasks/reorder",
      expect.objectContaining({ method: "PATCH" }),
    );
    const body = JSON.parse(f.mock.calls[0][1].body);
    expect(body.task_ids).toEqual(["task-1", "task-2"]);
  });
});
