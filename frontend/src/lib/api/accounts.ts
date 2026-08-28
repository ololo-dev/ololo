// Auth: register / login / logout.

import { request, type FetchLike } from "./core";
import type { RegisterBody, LoginBody } from "./types";

export function register(body: RegisterBody, opts: { fetch?: FetchLike } = {}) {
  return request<unknown>("/auth/register", {
    method: "POST",
    body,
    fetch: opts.fetch,
  });
}

export function login(body: LoginBody, opts: { fetch?: FetchLike } = {}) {
  return request<unknown>("/auth/login", {
    method: "POST",
    body,
    fetch: opts.fetch,
  });
}

export function logout(opts: { fetch?: FetchLike } = {}) {
  return request<void>("/auth/logout", { method: "POST", fetch: opts.fetch });
}
