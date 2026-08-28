import { beforeEach, describe, expect, it, vi } from "vitest";
import { request } from "./core";
import {
  listActiveSessions,
  listSessions,
  createSession,
  getSession,
  getSessionByCode,
  listProjectSessions,
  patchSession,
  deleteSession,
  listMembers,
  addMember,
  removeMember,
  joinSession,
  regenerateJoinCode,
  listTasks,
  listAgents,
  registerAgent,
} from "./sessions";
import type { RegisterAgentBody } from "./types";

vi.mock("./core", () => ({ request: vi.fn() }));

const mockRequest = vi.mocked(request);

describe("sessions api", () => {
  beforeEach(() => mockRequest.mockReset());

  it("listActiveSessions GETs the public endpoint and unwraps .sessions", async () => {
    mockRequest.mockResolvedValue({ sessions: [{ join_code: "ABC123" }] });
    const res = await listActiveSessions();
    expect(mockRequest).toHaveBeenCalledWith("/api/public/active-sessions", {
      fetch: undefined,
    });
    expect(res).toEqual([{ join_code: "ABC123" }]);
  });

  it("listSessions unwraps .sessions", async () => {
    mockRequest.mockResolvedValue({ sessions: [{ id: "s1" }] });
    const res = await listSessions();
    expect(mockRequest).toHaveBeenCalledWith("/api/sessions", { fetch: undefined });
    expect(res).toEqual([{ id: "s1" }]);
  });

  it("createSession POSTs name and project_id", async () => {
    mockRequest.mockResolvedValue({ id: "s1" });
    await createSession("Game", "p1");
    expect(mockRequest).toHaveBeenCalledWith("/api/sessions", {
      method: "POST",
      body: { name: "Game", project_id: "p1" },
      fetch: undefined,
    });
  });

  it("getSession GETs by id", async () => {
    mockRequest.mockResolvedValue({ id: "s1" });
    await getSession("s1");
    expect(mockRequest).toHaveBeenCalledWith("/api/sessions/s1", { fetch: undefined });
  });

  it("getSessionByCode GETs by code", async () => {
    mockRequest.mockResolvedValue({ id: "s1", join_code: "ABC" });
    await getSessionByCode("ABC");
    expect(mockRequest).toHaveBeenCalledWith("/api/sessions/by-code/ABC", { fetch: undefined });
  });

  it("listProjectSessions unwraps .sessions", async () => {
    mockRequest.mockResolvedValue({ sessions: [{ id: "s1" }] });
    const res = await listProjectSessions("p1");
    expect(mockRequest).toHaveBeenCalledWith("/api/projects/p1/sessions", { fetch: undefined });
    expect(res).toEqual([{ id: "s1" }]);
  });

  it("patchSession PATCHes with partial body", async () => {
    mockRequest.mockResolvedValue({ id: "s1", status: "running" });
    await patchSession("s1", { status: "running" });
    expect(mockRequest).toHaveBeenCalledWith("/api/sessions/s1", {
      method: "PATCH",
      body: { status: "running" },
      fetch: undefined,
    });
  });

  it("deleteSession DELETEs by id", async () => {
    mockRequest.mockResolvedValue(undefined);
    await deleteSession("s1");
    expect(mockRequest).toHaveBeenCalledWith("/api/sessions/s1", {
      method: "DELETE",
      fetch: undefined,
    });
  });

  it("listMembers unwraps .members", async () => {
    mockRequest.mockResolvedValue({ members: [{ user_id: "u1" }] });
    const res = await listMembers("s1");
    expect(mockRequest).toHaveBeenCalledWith("/api/sessions/s1/members", { fetch: undefined });
    expect(res).toEqual([{ user_id: "u1" }]);
  });

  it("addMember POSTs user_id and role", async () => {
    mockRequest.mockResolvedValue({ user_id: "u1", role: "participant" });
    await addMember("s1", "u1", "observer");
    expect(mockRequest).toHaveBeenCalledWith("/api/sessions/s1/members", {
      method: "POST",
      body: { user_id: "u1", role: "observer" },
      fetch: undefined,
    });
  });

  it("removeMember DELETEs member by user_id", async () => {
    mockRequest.mockResolvedValue(undefined);
    await removeMember("s1", "u1");
    expect(mockRequest).toHaveBeenCalledWith("/api/sessions/s1/members/u1", {
      method: "DELETE",
      fetch: undefined,
    });
  });

  it("joinSession POSTs the code", async () => {
    mockRequest.mockResolvedValue({ session_id: "s1" });
    await joinSession("ABC");
    expect(mockRequest).toHaveBeenCalledWith("/api/sessions/join", {
      method: "POST",
      body: { code: "ABC" },
      fetch: undefined,
    });
  });

  it("regenerateJoinCode POSTs and returns the new code", async () => {
    mockRequest.mockResolvedValue({ join_code: "XYZ" });
    const res = await regenerateJoinCode("s1");
    expect(mockRequest).toHaveBeenCalledWith("/api/sessions/s1/regenerate-code", {
      method: "POST",
      fetch: undefined,
    });
    expect(res).toEqual({ join_code: "XYZ" });
  });

  it("listTasks unwraps .tasks", async () => {
    mockRequest.mockResolvedValue({ tasks: [{ id: "t1" }] });
    const res = await listTasks("s1");
    expect(mockRequest).toHaveBeenCalledWith("/api/sessions/s1/tasks", { fetch: undefined });
    expect(res).toEqual([{ id: "t1" }]);
  });

  it("listAgents unwraps .agents", async () => {
    mockRequest.mockResolvedValue({ agents: [{ id: "a1" }] });
    const res = await listAgents("s1");
    expect(mockRequest).toHaveBeenCalledWith("/api/sessions/s1/agents", { fetch: undefined });
    expect(res).toEqual([{ id: "a1" }]);
  });

  it("registerAgent POSTs the body and returns agent + token", async () => {
    const body: RegisterAgentBody = {
      kind: "opencode",
      model: "m",
      display_name: "D",
    };
    mockRequest.mockResolvedValue({ agent: { id: "a1" }, observer_token: "tok" });
    const res = await registerAgent("s1", body);
    expect(mockRequest).toHaveBeenCalledWith("/api/sessions/s1/agents", {
      method: "POST",
      body,
      fetch: undefined,
    });
    expect(res).toEqual({ agent: { id: "a1" }, observer_token: "tok" });
  });

  it("encodes url segments", async () => {
    mockRequest.mockResolvedValue({ id: "s/1" });
    await getSession("s/1");
    expect(mockRequest).toHaveBeenCalledWith("/api/sessions/s%2F1", { fetch: undefined });
  });
});
