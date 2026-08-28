/**
 * Client-side access token manager.
 *
 * Persists the JWT in localStorage so JavaScript can read the `exp` claim
 * and schedule a silent refresh before the token expires, preventing the
 * 401 / missing-cookie errors that occur when the short-lived access cookie
 * runs out without being renewed.
 *
 * Only runs in the browser — all exports are no-ops or return null on the
 * server, so this module is safe to import anywhere in the SvelteKit tree.
 *
 * Flow:
 *   1. After login, +layout.svelte calls `initAuth()`.
 *   2. `initAuth()` loads the stored token or calls `silentRefresh()`.
 *   3. `silentRefresh()` hits `POST /auth/refresh` (sends the HttpOnly
 *      refresh cookie automatically) and stores the returned `access_token`.
 *   4. A timer fires `REFRESH_BEFORE_MS` before expiry and repeats step 3.
 *   5. `api.ts` reads the in-memory token and adds `Authorization: Bearer`
 *      to every client-side request; on 401 it calls `silentRefresh()` once
 *      and retries.
 *   6. `clearToken()` is called on logout.
 */

import { browser } from "$app/environment";

const TOKEN_KEY = "arena_access_token";
/** Refresh this many ms before the token's `exp` claim. */
const REFRESH_BEFORE_MS = 60_000;

// ---- module-level state (in-memory, survives SPA navigation) ----

let _token: string | null = null;
let _refreshTimer: ReturnType<typeof setTimeout> | null = null;
/**
 * In-flight refresh, shared by every caller.
 *
 * Refreshing rotates the server-side token, so a second concurrent POST
 * presents one that was just revoked. Several callers race by construction —
 * the expiry timer, `initAuth()` on navigation, and one `silentRefresh()` per
 * parallel request that 401s — so they are collapsed into a single request
 * whose result everyone awaits.
 */
let _refreshInFlight: Promise<boolean> | null = null;

// ---- helpers ----

function parseExpMs(jwt: string): number | null {
  try {
    const payload = jwt.split(".")[1];
    if (!payload) return null;
    const decoded = JSON.parse(atob(payload.replace(/-/g, "+").replace(/_/g, "/"))) as Record<
      string,
      unknown
    >;
    return typeof decoded.exp === "number" ? decoded.exp * 1000 : null;
  } catch {
    return null;
  }
}

function scheduleRefresh(jwt: string): void {
  if (_refreshTimer !== null) {
    clearTimeout(_refreshTimer);
    _refreshTimer = null;
  }
  const expMs = parseExpMs(jwt);
  if (expMs === null) return;
  const delayMs = expMs - Date.now() - REFRESH_BEFORE_MS;
  _refreshTimer = setTimeout(silentRefresh, Math.max(0, delayMs));
}

// ---- public API ----

/** Return the current in-memory access token (null when not authenticated). */
export function getToken(): string | null {
  return _token;
}

/** Store a new access token in memory and localStorage; schedule next refresh. */
export function setToken(jwt: string): void {
  _token = jwt;
  if (browser) {
    localStorage.setItem(TOKEN_KEY, jwt);
  }
  scheduleRefresh(jwt);
}

/** Clear the access token from memory and localStorage; cancel refresh timer. */
export function clearToken(): void {
  _token = null;
  if (browser) {
    localStorage.removeItem(TOKEN_KEY);
  }
  if (_refreshTimer !== null) {
    clearTimeout(_refreshTimer);
    _refreshTimer = null;
  }
}

/**
 * Call `POST /auth/refresh`, store the returned `access_token`, and return
 * whether it succeeded. The HttpOnly refresh cookie is sent automatically by
 * the browser (same-origin, `Path=/auth/refresh`).
 */
export async function silentRefresh(): Promise<boolean> {
  if (!browser) return false;
  if (_refreshInFlight !== null) return _refreshInFlight;
  _refreshInFlight = doRefresh().finally(() => {
    _refreshInFlight = null;
  });
  return _refreshInFlight;
}

async function doRefresh(): Promise<boolean> {
  try {
    const resp = await fetch("/auth/refresh", {
      method: "POST",
      credentials: "same-origin",
    });
    if (!resp.ok) {
      clearToken();
      return false;
    }
    const data = (await resp.json()) as { access_token?: string };
    if (data.access_token) {
      setToken(data.access_token);
      return true;
    }
    return false;
  } catch {
    // Network error — leave existing token; api.ts will surface the error.
    return false;
  }
}

/**
 * Initialize auth state on page load:
 *   - If localStorage has a token with enough remaining TTL, use it.
 *   - Otherwise perform a silent refresh to obtain a fresh token.
 *
 * Safe to call on every navigation; short-circuits when the in-memory
 * token is already loaded and valid.
 */
export async function initAuth(): Promise<void> {
  if (!browser) return;

  // Already have a valid in-memory token — nothing to do.
  if (_token !== null) {
    const expMs = parseExpMs(_token);
    if (expMs !== null && expMs - Date.now() > REFRESH_BEFORE_MS) return;
  }

  // Try localStorage first (survives hard-reloads within the same TTL window).
  const stored = localStorage.getItem(TOKEN_KEY);
  if (stored !== null) {
    const expMs = parseExpMs(stored);
    if (expMs !== null && expMs - Date.now() > REFRESH_BEFORE_MS) {
      _token = stored;
      scheduleRefresh(stored);
      return;
    }
  }

  // Token absent or expired — obtain a fresh one via the refresh cookie.
  await silentRefresh();
}
