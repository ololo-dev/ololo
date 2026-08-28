import { beforeEach, describe, expect, it, vi } from "vitest";
import { request } from "./core";
import { register, login, logout } from "./accounts";
import type { RegisterBody, LoginBody } from "./types";

vi.mock("./core", () => ({ request: vi.fn() }));

const mockRequest = vi.mocked(request);

describe("accounts api", () => {
  beforeEach(() => mockRequest.mockReset());

  it("register POSTs to /auth/register and returns the body", async () => {
    const body: RegisterBody = {
      email: "a@b.c",
      password: "pw",
      display_name: "A",
    };
    mockRequest.mockResolvedValue({ id: "u1" });
    const res = await register(body);
    expect(mockRequest).toHaveBeenCalledWith("/auth/register", {
      method: "POST",
      body,
      fetch: undefined,
    });
    expect(res).toEqual({ id: "u1" });
  });

  it("register forwards an injected fetch", async () => {
    const customFetch = (() => {}) as unknown as typeof fetch;
    await register({ email: "a@b.c", password: "pw", display_name: "A" }, { fetch: customFetch });
    expect(mockRequest).toHaveBeenCalledWith(
      "/auth/register",
      expect.objectContaining({ fetch: customFetch }),
    );
  });

  it("login POSTs to /auth/login", async () => {
    const body: LoginBody = { email: "a@b.c", password: "pw" };
    mockRequest.mockResolvedValue({ ok: true });
    await login(body);
    expect(mockRequest).toHaveBeenCalledWith("/auth/login", {
      method: "POST",
      body,
      fetch: undefined,
    });
  });

  it("logout POSTs to /auth/logout", async () => {
    mockRequest.mockResolvedValue(undefined);
    await logout();
    expect(mockRequest).toHaveBeenCalledWith("/auth/logout", {
      method: "POST",
      fetch: undefined,
    });
  });
});
