// Shared HTTP internals for the Arena REST client.

import { browser } from "$app/environment";
import { getToken, silentRefresh } from "../auth";
import { ApiError } from "./errors";

export type FetchLike = typeof fetch;

export interface Options {
  method?: "GET" | "POST" | "PATCH" | "PUT" | "DELETE";
  body?: unknown;
  fetch?: FetchLike;
  headers?: Record<string, string>;
}

export async function request<T>(path: string, opts: Options = {}): Promise<T> {
  // Client-side when no custom fetch is provided; use localStorage Bearer token.
  const clientSide = browser && !opts.fetch;
  const f = opts.fetch ?? fetch;

  const doFetch = async (): Promise<Response> => {
    const headers: Record<string, string> = { ...(opts.headers ?? {}) };
    if (opts.body !== undefined) headers["content-type"] = "application/json";
    if (clientSide) {
      const tok = getToken();
      if (tok) headers["authorization"] = `Bearer ${tok}`;
    }
    return f(path, {
      method: opts.method ?? "GET",
      credentials: "same-origin",
      headers: Object.keys(headers).length > 0 ? headers : undefined,
      body: opts.body !== undefined ? JSON.stringify(opts.body) : undefined,
    });
  };

  let resp = await doFetch();

  // On 401 in the browser, attempt a silent token refresh and retry once.
  if (resp.status === 401 && clientSide) {
    const refreshed = await silentRefresh();
    if (refreshed) {
      resp = await doFetch();
    }
  }

  if (resp.status === 204) {
    return undefined as T;
  }
  const text = await resp.text();
  let parsed: unknown = null;
  if (text.length > 0) {
    try {
      parsed = JSON.parse(text);
    } catch {
      parsed = text;
    }
  }
  if (!resp.ok) {
    const code =
      parsed && typeof parsed === "object" && parsed !== null && "error" in parsed
        ? String((parsed as { error: unknown }).error)
        : null;
    throw new ApiError(resp.status, code, parsed);
  }
  return parsed as T;
}
