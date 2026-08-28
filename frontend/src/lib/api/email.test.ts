import { beforeEach, describe, expect, it, vi } from "vitest";
import { request } from "./core";
import {
  getTurnstileConfig,
  forgotPassword,
  resetPassword,
  requestMagicLink,
  resendVerification,
} from "./email";

vi.mock("./core", () => ({ request: vi.fn() }));

const mockRequest = vi.mocked(request);

describe("email api", () => {
  beforeEach(() => mockRequest.mockReset());

  it("getTurnstileConfig GETs the public turnstile config", async () => {
    mockRequest.mockResolvedValue({ enabled: false, sitekey: null });
    const res = await getTurnstileConfig();
    expect(mockRequest).toHaveBeenCalledWith("/api/public/turnstile", { fetch: undefined });
    expect(res).toEqual({ enabled: false, sitekey: null });
  });

  it("forgotPassword sends only email when no token", async () => {
    mockRequest.mockResolvedValue(undefined);
    await forgotPassword("a@b.c");
    expect(mockRequest).toHaveBeenCalledWith("/auth/forgot-password", {
      method: "POST",
      body: { email: "a@b.c" },
    });
  });

  it("forgotPassword includes the turnstile token when given", async () => {
    mockRequest.mockResolvedValue(undefined);
    await forgotPassword("a@b.c", "tok");
    expect(mockRequest).toHaveBeenCalledWith("/auth/forgot-password", {
      method: "POST",
      body: { email: "a@b.c", turnstile_token: "tok" },
    });
  });

  it("resetPassword sends token and new password", async () => {
    mockRequest.mockResolvedValue(undefined);
    await resetPassword("rst", "newpw");
    expect(mockRequest).toHaveBeenCalledWith("/auth/reset-password", {
      method: "POST",
      body: { token: "rst", new_password: "newpw" },
    });
  });

  it("resetPassword forwards the turnstile token", async () => {
    mockRequest.mockResolvedValue(undefined);
    await resetPassword("rst", "newpw", "tt");
    expect(mockRequest).toHaveBeenCalledWith("/auth/reset-password", {
      method: "POST",
      body: { token: "rst", new_password: "newpw", turnstile_token: "tt" },
    });
  });

  it("requestMagicLink sends email", async () => {
    mockRequest.mockResolvedValue(undefined);
    await requestMagicLink("a@b.c");
    expect(mockRequest).toHaveBeenCalledWith("/auth/magic-link/request", {
      method: "POST",
      body: { email: "a@b.c" },
    });
  });

  it("requestMagicLink forwards the turnstile token", async () => {
    mockRequest.mockResolvedValue(undefined);
    await requestMagicLink("a@b.c", "tt");
    expect(mockRequest).toHaveBeenCalledWith("/auth/magic-link/request", {
      method: "POST",
      body: { email: "a@b.c", turnstile_token: "tt" },
    });
  });

  it("resendVerification POSTs to verify-email/resend", async () => {
    mockRequest.mockResolvedValue(undefined);
    await resendVerification();
    expect(mockRequest).toHaveBeenCalledWith("/auth/verify-email/resend", {
      method: "POST",
      fetch: undefined,
    });
  });
});
