import { describe, expect, it, vi } from "vitest";

const getSessionByCodeMock = vi.fn();

vi.mock("$lib/api", () => ({
  ApiError: class ApiError extends Error {
    status: number;

    constructor(status: number) {
      super(`api ${status}`);
      this.status = status;
    }
  },
  getSessionByCode: (...args: unknown[]) => getSessionByCodeMock(...args),
}));

describe("routes/s/[code]/lobby/+page.server.ts", () => {
  it("returns lobby session data and sets no-store header", async () => {
    const setHeaders = vi.fn();
    getSessionByCodeMock.mockResolvedValueOnce({
      id: "sess-1",
      join_code: "ABC123",
      state: "lobby",
      owner_id: null,
      project_id: "proj-1",
      project_name: "Project One",
      project_slug: "project-one",
    });

    const { load } = await import("./+page.server");
    const result = await load({
      params: { code: "ABC123" },
      fetch: vi.fn() as never,
      setHeaders,
    } as never);

    expect(result).toEqual({
      session: {
        id: "sess-1",
        join_code: "ABC123",
        state: "lobby",
        owner_id: null,
        project_id: "proj-1",
        project_name: "Project One",
        project_slug: "project-one",
      },
    });
    expect(setHeaders).toHaveBeenCalledWith({ "cache-control": "no-store" });
  });

  it("redirects to /s/[code] when phase is not lobby", async () => {
    getSessionByCodeMock.mockResolvedValueOnce({
      id: "sess-1",
      join_code: "ABC123",
      state: "running",
      owner_id: null,
      project_id: "proj-1",
      project_name: "Project One",
      project_slug: "project-one",
    });

    const { load } = await import("./+page.server");

    await expect(
      load({
        params: { code: "ABC123" },
        fetch: vi.fn() as never,
        setHeaders: vi.fn(),
      } as never),
    ).rejects.toMatchObject({ status: 307, location: "/s/ABC123" });
  });

  it("throws 404 on missing session", async () => {
    const { ApiError } = await import("$lib/api");
    getSessionByCodeMock.mockRejectedValueOnce(new ApiError(404, null, null));

    const { load } = await import("./+page.server");

    await expect(
      load({
        params: { code: "MISSING" },
        fetch: vi.fn() as never,
        setHeaders: vi.fn(),
      } as never),
    ).rejects.toMatchObject({ status: 404, body: { message: "Session not found" } });
  });
});
