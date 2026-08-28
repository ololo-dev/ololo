// Cookie passthrough: the SvelteKit server-side proxies forward upstream
// `Set-Cookie` headers to the browser. Cookies are HttpOnly so we cannot
// inspect their payloads; we only copy them verbatim, preserving every
// directive the backend emitted.

import type { Cookies } from "@sveltejs/kit";

interface ParsedCookie {
  name: string;
  value: string;
  options: {
    path?: string;
    domain?: string;
    expires?: Date;
    maxAge?: number;
    httpOnly?: boolean;
    secure?: boolean;
    sameSite?: "strict" | "lax" | "none";
  };
}

/** Split a multi-Set-Cookie header. `Headers#getSetCookie()` is preferred when present. */
export function extractSetCookies(headers: Headers): string[] {
  const anyHeaders = headers as unknown as { getSetCookie?: () => string[] };
  if (typeof anyHeaders.getSetCookie === "function") {
    return anyHeaders.getSetCookie();
  }
  // Fallback: combined header. Naive split on ", " is unsafe (Expires
  // attribute contains commas). Best-effort split on `,(?= [^ ;]+=)` —
  // a comma followed by a token then `=`.
  const raw = headers.get("set-cookie");
  if (!raw) return [];
  return raw.split(/,(?=\s*[A-Za-z0-9!#$%&'*+\-.^_`|~]+=)/);
}

export function parseSetCookie(input: string): ParsedCookie | null {
  const parts = input.split(";").map((s) => s.trim());
  if (parts.length === 0) return null;
  const first = parts[0];
  const eq = first.indexOf("=");
  if (eq < 0) return null;
  const name = first.slice(0, eq).trim();
  const value = decodeURIComponent(first.slice(eq + 1).trim());
  const options: ParsedCookie["options"] = {};
  for (let i = 1; i < parts.length; i++) {
    const p = parts[i];
    const [k, v] = p.split("=", 2);
    const lower = k.toLowerCase();
    if (lower === "path") options.path = v;
    else if (lower === "domain") options.domain = v;
    else if (lower === "expires" && v) options.expires = new Date(v);
    else if (lower === "max-age" && v) options.maxAge = Number(v);
    else if (lower === "httponly") options.httpOnly = true;
    else if (lower === "secure") options.secure = true;
    else if (lower === "samesite" && v) {
      const s = v.toLowerCase();
      if (s === "strict" || s === "lax" || s === "none") options.sameSite = s;
    }
  }
  return { name, value, options };
}

/** Forward every Set-Cookie from `upstream` onto SvelteKit's `cookies`. */
export function forwardSetCookies(upstream: Response, cookies: Cookies): void {
  for (const raw of extractSetCookies(upstream.headers)) {
    const parsed = parseSetCookie(raw);
    if (!parsed) continue;
    const opts: Parameters<Cookies["set"]>[2] = {
      path: parsed.options.path ?? "/",
    };
    if (parsed.options.domain) opts.domain = parsed.options.domain;
    if (parsed.options.expires) opts.expires = parsed.options.expires;
    if (parsed.options.maxAge !== undefined) opts.maxAge = parsed.options.maxAge;
    if (parsed.options.httpOnly !== undefined) opts.httpOnly = parsed.options.httpOnly;
    if (parsed.options.secure !== undefined) opts.secure = parsed.options.secure;
    if (parsed.options.sameSite) opts.sameSite = parsed.options.sameSite;
    if (parsed.value === "" && parsed.options.maxAge === 0) {
      cookies.delete(parsed.name, { path: opts.path ?? "/" });
    } else {
      cookies.set(parsed.name, parsed.value, opts);
    }
  }
}
