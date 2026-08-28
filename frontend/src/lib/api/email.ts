// Email / password reset / magic link auth flows (public).

import { request, type FetchLike } from "./core";
import type { TurnstileConfigResponse } from "./types";

export function getTurnstileConfig(opts: { fetch?: FetchLike } = {}) {
  return request<TurnstileConfigResponse>("/api/public/turnstile", {
    fetch: opts.fetch,
  });
}

export function forgotPassword(email: string, turnstileToken?: string) {
  return request<void>("/auth/forgot-password", {
    method: "POST",
    body: turnstileToken ? { email, turnstile_token: turnstileToken } : { email },
  });
}

export function resetPassword(token: string, newPassword: string, turnstileToken?: string) {
  return request<void>("/auth/reset-password", {
    method: "POST",
    body: turnstileToken
      ? { token, new_password: newPassword, turnstile_token: turnstileToken }
      : { token, new_password: newPassword },
  });
}

export function requestMagicLink(email: string, turnstileToken?: string) {
  return request<void>("/auth/magic-link/request", {
    method: "POST",
    body: turnstileToken ? { email, turnstile_token: turnstileToken } : { email },
  });
}

export function resendVerification(opts: { fetch?: FetchLike } = {}) {
  return request<void>("/auth/verify-email/resend", {
    method: "POST",
    fetch: opts.fetch,
  });
}
