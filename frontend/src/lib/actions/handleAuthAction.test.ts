import { describe, it, expect, vi } from "vitest";

const forwardSetCookies = vi.fn();
vi.mock("$lib/cookies", () => ({
  forwardSetCookies: (...args: unknown[]) => forwardSetCookies(...args),
}));

import { handleAuthAction } from "./handleAuthAction";

function makeResponse(ok: boolean, status: number, json: unknown): Response {
  return {
    ok,
    status,
    json: async () => json,
    headers: new Headers(),
  } as unknown as Response;
}

function fakeCookies() {
  return { set: vi.fn(), delete: vi.fn() } as unknown as import("@sveltejs/kit").Cookies;
}

describe("handleAuthAction", () => {
  it("forwards Set-Cookie and returns a fail with the upstream error code", async () => {
    const cookies = fakeCookies();
    const fetchMock = vi.fn(async () => makeResponse(false, 401, { error: "x" }));

    const result = (await handleAuthAction({
      fetch: fetchMock,
      cookies,
      endpoint: "/auth/login",
      body: { email: "a@b.c", password: "p" },
      forwardCookies: true,
      defaultError: "login_failed",
      failFields: { email: "a@b.c" },
    }))!;

    expect(forwardSetCookies).toHaveBeenCalledTimes(1);
    expect(forwardSetCookies).toHaveBeenCalledWith(expect.anything(), cookies);
    expect(fetchMock).toHaveBeenCalledWith(
      "/auth/login",
      expect.objectContaining({ method: "POST" }),
    );
    expect(result.status).toBe(401);
    const data = (result as { data?: Record<string, unknown> }).data ?? {};
    expect(data.error).toBe("x");
    expect(data.email).toBe("a@b.c");
  });

  it("uses the default error code when the upstream body has no error field", async () => {
    const fetchMock = vi.fn(async () => makeResponse(false, 500, {}));
    const result = (await handleAuthAction({
      fetch: fetchMock,
      endpoint: "/auth/register",
      body: { email: "a@b.c", password: "p", display_name: "A" },
      defaultError: "register_failed",
    }))!;
    expect(result.status).toBe(500);
    const data = (result as { data?: Record<string, unknown> }).data ?? {};
    expect(data.error).toBe("register_failed");
  });

  it("throws a redirect when next is given", async () => {
    const fetchMock = vi.fn(async () => makeResponse(true, 200, {}));
    await expect(
      handleAuthAction({
        fetch: fetchMock,
        endpoint: "/auth/login",
        body: { email: "a@b.c", password: "p" },
        next: "/dashboard",
      }),
    ).rejects.toMatchObject({ status: 303, location: "/dashboard" });
  });

  it("returns the success object when returnOnSuccess is given", async () => {
    const fetchMock = vi.fn(async () => makeResponse(true, 200, {}));
    const result = await handleAuthAction({
      fetch: fetchMock,
      endpoint: "/auth/forgot-password",
      body: { email: "a@b.c" },
      defaultError: "captcha_error",
      returnOnSuccess: { success: true },
    });
    expect((result as { success?: boolean }).success).toBe(true);
  });

  it("runs beforeRedirect before throwing", async () => {
    const cookies = fakeCookies();
    const fetchMock = vi.fn(async () => makeResponse(true, 200, {}));
    const beforeRedirect = vi.fn();
    await expect(
      handleAuthAction({
        fetch: fetchMock,
        cookies,
        endpoint: "/auth/logout",
        forwardCookies: true,
        next: "/",
        beforeRedirect,
      }),
    ).rejects.toMatchObject({ status: 303, location: "/" });
    expect(beforeRedirect).toHaveBeenCalledTimes(1);
  });
});
