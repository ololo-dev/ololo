import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { request } from "./core";
import { ApiError } from "./errors";
import {
  getMe,
  getSessionReport,
  patchMe,
  getAvatarAuth,
  changePassword,
  listUserPats,
  revokeUserPat,
  getSettings,
  updateSetting,
  getCategories,
  createCategory,
  deleteCategory,
  reorderCategories,
  getAdminUsers,
  createAdminUser,
  updateAdminUser,
  deleteAdminUser,
  getGameServers,
  patchGameServer,
  getEmailTemplates,
  updateEmailTemplate,
  updateEmailSetting,
  getPublicProfile,
  getPublicSessions,
} from "./users";

vi.mock("./core", () => ({ request: vi.fn() }));

const mockRequest = vi.mocked(request);

describe("users api (request-based)", () => {
  beforeEach(() => mockRequest.mockReset());

  it("getMe GETs /api/users/me", async () => {
    mockRequest.mockResolvedValue({ id: "u1" });
    await getMe();
    expect(mockRequest).toHaveBeenCalledWith("/api/users/me", { fetch: undefined });
  });

  it("getSessionReport GETs the report", async () => {
    mockRequest.mockResolvedValue({
      session_id: "s1",
      status: "finished",
      leaderboard: [],
      timeline: [],
    });
    await getSessionReport("s1");
    expect(mockRequest).toHaveBeenCalledWith("/api/sessions/s1/report", { fetch: undefined });
  });

  it("patchMe PATCHes /api/users/me", async () => {
    mockRequest.mockResolvedValue({ id: "u1" });
    await patchMe({ display_name: "D" });
    expect(mockRequest).toHaveBeenCalledWith("/api/users/me", {
      method: "PATCH",
      body: { display_name: "D" },
      fetch: undefined,
    });
  });

  it("getAvatarAuth GETs avatar-auth", async () => {
    mockRequest.mockResolvedValue({ token: "t" });
    await getAvatarAuth();
    expect(mockRequest).toHaveBeenCalledWith("/api/users/me/avatar-auth", { fetch: undefined });
  });

  it("changePassword POSTs current/new", async () => {
    mockRequest.mockResolvedValue({ ok: true });
    await changePassword({ current_password: "a", new_password: "b" });
    expect(mockRequest).toHaveBeenCalledWith("/api/users/me/password", {
      method: "POST",
      body: { current_password: "a", new_password: "b" },
      fetch: undefined,
    });
  });

  it("listUserPats unwraps .pats", async () => {
    mockRequest.mockResolvedValue({ pats: [{ id: "p1" }] });
    const res = await listUserPats();
    expect(mockRequest).toHaveBeenCalledWith("/api/users/me/pats", { fetch: undefined });
    expect(res).toEqual([{ id: "p1" }]);
  });

  it("revokeUserPat DELETEs by id", async () => {
    mockRequest.mockResolvedValue(undefined);
    await revokeUserPat("p1");
    expect(mockRequest).toHaveBeenCalledWith("/api/users/me/pats/p1", {
      method: "DELETE",
      fetch: undefined,
    });
  });

  it("getSettings GETs admin settings", async () => {
    mockRequest.mockResolvedValue({ ai_model: "m" });
    await getSettings();
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/settings", { fetch: undefined });
  });

  it("updateSetting PUTs key/value", async () => {
    mockRequest.mockResolvedValue({ ok: true });
    await updateSetting("k", "v");
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/settings", {
      method: "PUT",
      body: { key: "k", value: "v" },
      fetch: undefined,
    });
  });

  it("getCategories unwraps .categories", async () => {
    mockRequest.mockResolvedValue({ categories: [{ id: 1, name: "c" }] });
    const res = await getCategories();
    expect(mockRequest).toHaveBeenCalledWith("/api/categories", { fetch: undefined });
    expect(res).toEqual([{ id: 1, name: "c" }]);
  });

  it("createCategory POSTs name", async () => {
    mockRequest.mockResolvedValue({ category: { id: 1, name: "c" } });
    await createCategory("c");
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/categories", {
      method: "POST",
      body: { name: "c" },
      fetch: undefined,
    });
  });

  it("deleteCategory DELETEs by id", async () => {
    mockRequest.mockResolvedValue({ ok: true });
    await deleteCategory(2);
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/categories/2", {
      method: "DELETE",
      fetch: undefined,
    });
  });

  it("reorderCategories PATCHes ids", async () => {
    mockRequest.mockResolvedValue({ ok: true });
    await reorderCategories([3, 1, 2]);
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/categories/reorder", {
      method: "PATCH",
      body: { ids: [3, 1, 2] },
      fetch: undefined,
    });
  });

  it("getAdminUsers unwraps .users", async () => {
    mockRequest.mockResolvedValue({ users: [{ id: "u1" }] });
    const res = await getAdminUsers();
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/users", { fetch: undefined });
    expect(res).toEqual([{ id: "u1" }]);
  });

  it("createAdminUser POSTs the body", async () => {
    const body = { email: "a@b.c", display_name: "D", password: "pw", is_admin: false };
    mockRequest.mockResolvedValue({ user: { id: "u1" } });
    await createAdminUser(body);
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/users", {
      method: "POST",
      body,
    });
  });

  it("updateAdminUser PATCHes by id", async () => {
    mockRequest.mockResolvedValue({ user: { id: "u1" } });
    await updateAdminUser("u1", { display_name: "D2" });
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/users/u1", {
      method: "PATCH",
      body: { display_name: "D2" },
    });
  });

  it("deleteAdminUser DELETEs by id", async () => {
    mockRequest.mockResolvedValue({ ok: true });
    await deleteAdminUser("u1");
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/users/u1", {
      method: "DELETE",
    });
  });

  it("getGameServers unwraps .servers", async () => {
    mockRequest.mockResolvedValue({ servers: [{ id: "g1" }] });
    const res = await getGameServers();
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/game-servers", { fetch: undefined });
    expect(res).toEqual([{ id: "g1" }]);
  });

  it("patchGameServer PATCHes by id", async () => {
    mockRequest.mockResolvedValue({ server: { id: "g1" } });
    await patchGameServer("g1", { status: "offline" });
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/game-servers/g1", {
      method: "PATCH",
      body: { status: "offline" },
    });
  });

  it("getEmailTemplates GETs templates", async () => {
    mockRequest.mockResolvedValue([
      { type: "t", subject: "s", body_html: "", body_text: "", updated_at: "" },
    ]);
    await getEmailTemplates();
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/email-templates", { fetch: undefined });
  });

  it("updateEmailTemplate PUTs by type", async () => {
    mockRequest.mockResolvedValue(undefined);
    await updateEmailTemplate("t", { subject: "s", body_html: "h", body_text: "x" });
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/email-templates/t", {
      method: "PUT",
      body: { subject: "s", body_html: "h", body_text: "x" },
      fetch: undefined,
    });
  });

  it("updateEmailSetting PUTs key/value", async () => {
    mockRequest.mockResolvedValue(undefined);
    await updateEmailSetting("k", "v");
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/settings", {
      method: "PUT",
      body: { key: "k", value: "v" },
      fetch: undefined,
    });
  });
});

describe("users api (public fetch-based)", () => {
  let fetchMock: ReturnType<typeof vi.fn>;
  beforeEach(() => {
    fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
  });
  afterEach(() => vi.unstubAllGlobals());

  it("getPublicProfile returns json on success", async () => {
    const profile = { username: "u", display_name: "D", avatar_url: null };
    fetchMock.mockResolvedValue({
      ok: true,
      json: async () => profile,
    });
    const res = await getPublicProfile("u");
    expect(fetchMock).toHaveBeenCalledWith("/api/users/by-username/u", {
      credentials: "omit",
    });
    expect(res).toEqual(profile);
  });

  it("getPublicProfile throws ApiError on failure", async () => {
    fetchMock.mockResolvedValue({
      ok: false,
      status: 404,
      text: async () => JSON.stringify({ error: "not_found" }),
    });
    await expect(getPublicProfile("u")).rejects.toThrow(ApiError);
    await expect(getPublicProfile("u")).rejects.toMatchObject({
      status: 404,
      code: "not_found",
    });
  });

  it("getPublicSessions returns parsed json with query params", async () => {
    const resp = { sessions: [], total: 0, page: 2, per_page: 10 };
    fetchMock.mockResolvedValue({ ok: true, json: async () => resp });
    const res = await getPublicSessions("u", 2, 10);
    expect(fetchMock).toHaveBeenCalledWith("/api/users/by-username/u/sessions?page=2&per_page=10", {
      credentials: "omit",
    });
    expect(res).toEqual(resp);
  });

  it("getPublicSessions throws ApiError on failure", async () => {
    fetchMock.mockResolvedValue({
      ok: false,
      status: 500,
      text: async () => "boom",
    });
    await expect(getPublicSessions("u")).rejects.toThrow(ApiError);
  });
});
