import type { Handle, HandleFetch } from "@sveltejs/kit";
import { redirect } from "@sveltejs/kit";
import { env } from "$env/dynamic/private";

const ACCESS_COOKIE = "arena_access";
const REFRESH_COOKIE = "arena_refresh";
const PROTECTED_PREFIXES = ["/s"];

// Path prefixes owned by the Rust API server, not this SvelteKit app.
const API_PREFIXES = ["/api/", "/auth/", "/git/"];

// Cookie attributes mirror the server's (see api::users::auth). Path=/ lets the
// refresh cookie reach these hooks on ordinary navigations.
const AUTH_COOKIE_OPTS = {
  path: "/",
  httpOnly: true,
  secure: true,
  sameSite: "lax",
} as const;

// Paths that are publicly accessible even though they fall under a protected prefix.
const PUBLIC_PATH_PATTERNS: RegExp[] = [
  /^\/s\/[^/]+$/, // /s/[code]       — session spectator
  /^\/s\/[^/]+\/lobby$/, // /s/[code]/lobby — session lobby
];

/**
 * Cookie-presence gate. The HttpOnly access cookie is verified by the
 * backend on every API call; we only check existence here so unauthenticated
 * page loads are short-circuited to /login. Stale/forged cookies will still
 * receive a 401 from the API and the component layer surfaces a re-login
 * prompt.
 */
/**
 * Decode the `sub` claim from a JWT without verifying the signature.
 * The backend validates the token on every API request; we only need the
 * user-id here so that page server loads can derive ownership flags.
 * Returns `null` when the token is absent or malformed.
 */
function jwtClaims(token: string | undefined): Record<string, unknown> | null {
  if (!token) return null;
  try {
    const payload = token.split(".")[1];
    if (!payload) return null;
    const json = atob(payload.replace(/-/g, "+").replace(/_/g, "/"));
    return JSON.parse(json) as Record<string, unknown>;
  } catch {
    return null;
  }
}

function jwtSub(token: string | undefined): string | null {
  const claims = jwtClaims(token);
  return claims && typeof claims.sub === "string" ? claims.sub : null;
}

/**
 * Whether the access token is absent or has expired (with a small clock-skew
 * margin). Signature is NOT verified here — the backend does that on every API
 * call; this only decides whether a server-side refresh should be attempted.
 */
function accessTokenExpired(token: string | undefined): boolean {
  const claims = jwtClaims(token);
  if (!claims || typeof claims.exp !== "number") return true;
  return Date.now() / 1000 >= claims.exp - 5;
}

function parseSetCookie(sc: string): { name: string; value: string; maxAge?: number } | null {
  const [pair, ...attrs] = sc.split(";");
  const eq = pair.indexOf("=");
  if (eq < 0) return null;
  const name = pair.slice(0, eq).trim();
  const value = pair.slice(eq + 1).trim();
  let maxAge: number | undefined;
  for (const a of attrs) {
    const [k, v] = a.split("=");
    if (k.trim().toLowerCase() === "max-age" && v !== undefined) {
      const n = Number(v.trim());
      if (Number.isFinite(n)) maxAge = n;
    }
  }
  return { name, value, maxAge };
}

/**
 * Server-side silent refresh: the access token is short-lived and the client
 * refreshes it proactively, but a server-rendered navigation made after it
 * lapses has no client to refresh — its page load would call an authed API,
 * get 401 and throw a 500. Mirror the client here: POST /auth/refresh with the
 * (now Path=/) refresh cookie, then adopt the rotated access+refresh cookies for
 * this request and forward them to the browser. Returns the new access token,
 * or null when refresh is not possible (no/expired refresh token).
 */
async function serverSilentRefresh(event: Parameters<Handle>[0]["event"]): Promise<string | null> {
  try {
    const resp = await event.fetch("/auth/refresh", { method: "POST" });
    if (!resp.ok) return null;
    const setCookies = resp.headers.getSetCookie?.() ?? [];
    for (const sc of setCookies) {
      const parsed = parseSetCookie(sc);
      if (parsed && (parsed.name === ACCESS_COOKIE || parsed.name === REFRESH_COOKIE)) {
        event.cookies.set(parsed.name, parsed.value, {
          ...AUTH_COOKIE_OPTS,
          maxAge: parsed.maxAge,
        });
      }
    }
    const data = (await resp.json().catch(() => null)) as { access_token?: unknown } | null;
    if (typeof data?.access_token === "string") return data.access_token;
    return event.cookies.get(ACCESS_COOKIE) ?? null;
  } catch {
    return null;
  }
}

export const handle: Handle = async ({ event, resolve }) => {
  // RFC 9727 API catalog for agent discovery — a linkset (RFC 9264) pointing
  // agents at the API and its human documentation. Advertised via the Link
  // response header below.
  if (event.url.pathname === "/.well-known/api-catalog") {
    const origin = event.url.origin;
    const catalog = {
      linkset: [
        {
          anchor: `${origin}/api`,
          "service-doc": [{ href: `${origin}/documentation`, type: "text/html" }],
        },
      ],
    };
    return new Response(JSON.stringify(catalog), {
      headers: {
        "content-type": "application/linkset+json",
        "cache-control": "public, max-age=86400",
      },
    });
  }

  // Chrome DevTools unconditionally probes other .well-known paths on every
  // local server. Return 404 silently rather than letting SvelteKit log a
  // SvelteKitError.
  if (event.url.pathname.startsWith("/.well-known/")) {
    return new Response(null, { status: 404 });
  }

  let token = event.cookies.get(ACCESS_COOKIE);
  // Silent-refresh a lapsed access token before any page load runs, so an
  // SSR navigation after the ~15-min token expiry re-authenticates instead of
  // 500-ing (page loads call authed APIs and have no client to refresh).
  //
  // NEVER for API-owned paths: in dev (no INTERNAL_API_ORIGIN rewrite) a
  // same-origin `event.fetch("/auth/refresh")` is dispatched in-process and
  // re-enters this handle — with an expired access + present refresh cookie
  // that recursed unboundedly until the dev server OOM'd. API paths are not
  // SvelteKit pages; they need no page-load refresh here.
  const isApiOwned = API_PREFIXES.some((p) => event.url.pathname.startsWith(p));
  if (!isApiOwned && accessTokenExpired(token) && event.cookies.get(REFRESH_COOKIE)) {
    const refreshed = await serverSilentRefresh(event);
    if (refreshed) token = refreshed;
  }
  const hasAccess = !!token && !accessTokenExpired(token);
  event.locals.isAuthenticated = hasAccess;
  event.locals.userId = jwtSub(token);

  const path = event.url.pathname;
  const isProtected = PROTECTED_PREFIXES.some((p) => path === p || path.startsWith(`${p}/`));
  const isPublic = PUBLIC_PATH_PATTERNS.some((re) => re.test(path));

  if (isProtected && !isPublic && !hasAccess) {
    const next = encodeURIComponent(path + event.url.search);
    throw redirect(303, `/login?next=${next}`);
  }

  // The Ukrainian pages are a real language version, not a translated
  // fragment — screen readers and the acquirer's review both read the
  // document language, so swap it for those routes only.
  const isUkrainian = event.url.pathname === "/ua" || event.url.pathname.startsWith("/ua/");
  const response = await resolve(event, {
    transformPageChunk: isUkrainian
      ? ({ html }) => html.replace('<html lang="en">', '<html lang="uk">')
      : undefined,
  });
  // Baseline security headers. A full CSP is deliberately omitted: SvelteKit
  // hydrates via inline scripts and the homepage embeds JSON-LD inline, so a
  // safe policy needs nonce plumbing — not worth it yet.
  response.headers.set("Strict-Transport-Security", "max-age=63072000; includeSubDomains");
  response.headers.set("X-Frame-Options", "SAMEORIGIN");
  response.headers.set("X-Content-Type-Options", "nosniff");
  response.headers.set("Referrer-Policy", "strict-origin-when-cross-origin");
  response.headers.set("Cross-Origin-Opener-Policy", "same-origin-allow-popups");
  // Agent discovery (RFC 8288 Link relations): advertise the API catalog and
  // the human-readable docs on every server-rendered page.
  response.headers.set(
    "Link",
    '</.well-known/api-catalog>; rel="api-catalog", </documentation>; rel="service-doc"',
  );
  return response;
};

/**
 * SvelteKit resolves same-origin server-side `fetch` calls against its own
 * router in-process — they never reach the network, so relative API calls
 * from loads/actions 404 in production (in dev the Vite proxy handles them
 * at the HTTP layer instead). When INTERNAL_API_ORIGIN is set (the compose
 * deployment sets http://server:8080), rewrite API-owned paths to it and
 * forward the caller's cookies, which SvelteKit only auto-attaches for
 * same-site hosts.
 */
export const handleFetch: HandleFetch = async ({ event, request, fetch }) => {
  const internal = env.INTERNAL_API_ORIGIN;
  if (internal) {
    const url = new URL(request.url);
    if (url.origin === event.url.origin && API_PREFIXES.some((p) => url.pathname.startsWith(p))) {
      request = new Request(internal + url.pathname + url.search, request);
      // Forward the caller's IP: the API rate-limits by x-forwarded-for, and
      // without this every SSR request shares the frontend container's IP —
      // one 429 bucket for every viewer of every page (seen as 500s on the
      // session dashboard mid-session).
      const xff = event.request.headers.get("x-forwarded-for") ?? event.getClientAddress();
      if (xff) request.headers.set("x-forwarded-for", xff);
      // Behind Cloudflare, Traefik strips x-forwarded-for as untrusted, so
      // CF-Connecting-IP is the only header that still names the real client
      // — and the API prefers it for the same reason.
      const cfip = event.request.headers.get("cf-connecting-ip");
      if (cfip) request.headers.set("cf-connecting-ip", cfip);
      // Forward cookies from event.cookies (not the raw request header) so a
      // token refreshed in `handle` is the one sent to the API this request.
      const cookie = event.cookies
        .getAll()
        .map((c) => `${c.name}=${c.value}`)
        .join("; ");
      if (cookie) request.headers.set("cookie", cookie);
    }
  }
  return fetch(request);
};
