import { describe, expect, it, vi } from "vitest";

const getAdminUsers = vi.fn();
const listProjects = vi.fn();
const getCategories = vi.fn();
const listJudges = vi.fn();
const listAdminSessions = vi.fn();
vi.mock("$lib/api", () => ({
  getAdminUsers: (...args: unknown[]) => getAdminUsers(...args),
  listProjects: (...args: unknown[]) => listProjects(...args),
  getCategories: (...args: unknown[]) => getCategories(...args),
  listJudges: (...args: unknown[]) => listJudges(...args),
  listAdminSessions: (...args: unknown[]) => listAdminSessions(...args),
}));

import { load } from "../routes/settings/+layout.server";

function fakeEvent(overrides: Partial<{ isAuthenticated: boolean }> = {}) {
  return {
    locals: { isAuthenticated: true, ...overrides },
    parent: async () => ({ isAdmin: true }),
    fetch: vi.fn(),
  };
}

describe("settings/+layout.server load (auth guard)", () => {
  it("redirects unauthenticated users to /login", async () => {
    // parent() must not be reached for an unauthenticated user.
    const parent = vi.fn(async () => ({ isAdmin: true }));
    await expect(
      load({ locals: { isAuthenticated: false }, parent, fetch: vi.fn() } as never),
    ).rejects.toMatchObject({ status: 303, location: "/login?next=/settings" });
    expect(parent).not.toHaveBeenCalled();
  });

  it("proceeds and returns counts when authenticated and admin", async () => {
    getAdminUsers.mockResolvedValueOnce([{ id: "u1" }, { id: "u2" }]);
    listProjects.mockResolvedValueOnce([{ id: "p1" }]);
    getCategories.mockResolvedValueOnce([{ id: 1 }, { id: 2 }, { id: 3 }]);
    listJudges.mockResolvedValueOnce([{ id: "j1" }]);
    listAdminSessions.mockResolvedValueOnce({ total: 42 });
    const result = await load(fakeEvent() as never);
    expect(getAdminUsers).toHaveBeenCalledTimes(1);
    expect(listProjects).toHaveBeenCalledWith(true, expect.anything());
    // The badge only needs the total, so the page asked for is the smallest
    // one rather than every session on the instance.
    expect(listAdminSessions).toHaveBeenCalledWith({ per_page: 1 }, expect.anything());
    expect(result).toEqual({
      userCount: 2,
      projectCount: 1,
      categoryCount: 3,
      judgeCount: 1,
      sessionCount: 42,
    });
  });

  it("degrades a failing count to zero instead of breaking the shell", async () => {
    getAdminUsers.mockResolvedValueOnce([{ id: "u1" }]);
    listProjects.mockResolvedValueOnce([]);
    getCategories.mockRejectedValueOnce(new Error("boom"));
    listJudges.mockResolvedValueOnce([{ id: "j1" }]);
    listAdminSessions.mockRejectedValueOnce(new Error("boom"));
    const result = await load(fakeEvent() as never);
    expect(result).toEqual({
      userCount: 1,
      projectCount: 0,
      categoryCount: 0,
      judgeCount: 1,
      sessionCount: 0,
    });
  });
});
