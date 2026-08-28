import { browser, dev } from "$app/environment";
import { fail, redirect } from "@sveltejs/kit";
import type { ActionFailure, Cookies } from "@sveltejs/kit";
import { forwardSetCookies } from "$lib/cookies";
import { getTurnstileConfig } from "$lib/api";

if (browser && !dev) {
  throw new Error("handleAuthAction is server-only and must not run in the browser.");
}

export interface HandleAuthActionArgs<
  F extends Record<string, unknown> = Record<string, unknown>,
  S extends Record<string, unknown> | undefined = Record<string, unknown> | undefined,
> {
  fetch: typeof fetch;
  cookies?: Cookies;
  endpoint: string;
  method?: string;
  body?: unknown;
  contentType?: string;
  forwardCookies?: boolean;
  failFields?: F;
  defaultError?: string;
  parseErrorFromJson?: boolean;
  next?: string;
  returnOnSuccess?: S;
  redirectOnFailure?: boolean;
  beforeRedirect?: () => void;
}

export async function handleAuthAction<
  F extends Record<string, unknown> = Record<string, unknown>,
  S extends Record<string, unknown> | undefined = undefined,
>(args: HandleAuthActionArgs<F, S>): Promise<ActionFailure<F & { error: string }> | S | undefined> {
  const {
    fetch,
    cookies,
    endpoint,
    method = "POST",
    body,
    contentType = "application/json",
    forwardCookies = false,
    failFields = {} as F,
    defaultError = "auth_failed",
    parseErrorFromJson = true,
    next,
    returnOnSuccess,
    redirectOnFailure = false,
    beforeRedirect,
  } = args;

  const init: RequestInit = { method };
  if (body !== undefined) {
    init.headers = { "content-type": contentType };
    init.body = contentType === "application/json" ? JSON.stringify(body) : String(body);
  }

  const upstream = await fetch(endpoint, init);

  if (forwardCookies && cookies) {
    forwardSetCookies(upstream, cookies);
  }

  if (!upstream.ok && !redirectOnFailure) {
    let code = defaultError;
    if (parseErrorFromJson) {
      try {
        const respBody = (await upstream.json()) as { error?: string };
        if (respBody?.error) code = respBody.error;
      } catch {
        // ignore malformed bodies
      }
    }
    return fail(upstream.status, { error: code, ...failFields } as F & { error: string });
  }

  if (next !== undefined) {
    beforeRedirect?.();
    throw redirect(303, next);
  }

  if (returnOnSuccess) {
    return returnOnSuccess;
  }

  return undefined;
}

/// Shared `load` for auth pages that render a Turnstile CAPTCHA widget.
export async function loadTurnstile({ fetch }: { fetch: typeof globalThis.fetch }) {
  let turnstile: { enabled: boolean; sitekey: string | null } = { enabled: false, sitekey: null };
  try {
    turnstile = await getTurnstileConfig({ fetch });
  } catch {
    // ignore — CAPTCHA disabled
  }
  return { turnstile };
}

/// Form action for email-only auth requests (forgot-password, magic-link):
/// validates the email field, threads the optional Turnstile token, and
/// POSTs to `endpoint`.
export function emailRequestAction(endpoint: string) {
  return async ({ request, fetch }: { request: Request; fetch: typeof globalThis.fetch }) => {
    const data = await request.formData();
    const email = String(data.get("email") ?? "").trim();
    const turnstileToken = String(data.get("turnstile_token") ?? "");
    if (!email) {
      return fail(422, { error: "missing_fields", email });
    }
    const body: Record<string, string> = { email };
    if (turnstileToken) body.turnstile_token = turnstileToken;
    return handleAuthAction({
      fetch,
      endpoint,
      body,
      defaultError: "captcha_error",
      returnOnSuccess: { success: true },
    });
  };
}
