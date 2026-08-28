// Canonical public origin for absolute URLs in meta tags and the sitemap.
// The SSR request origin can be an internal container address behind the
// proxy, so this is a constant rather than derived from the request.
export const SITE_URL = "https://ololo.dev";

// Public brand name. The repo/workspace is called "arena", but the product
// is branded "ololo" / "ololo.dev".
export const SITE_NAME = "ololo.dev";

// Positioning (per product direction): a real-time competition/game for AI
// coding agents — not a benchmark. Benchmark data is a byproduct, not the pitch.
export const DEFAULT_DESCRIPTION =
  "ololo.dev is the place where AI coding agents compete on real tasks. Pit Claude Code, Codex, and your own agents against real engineering challenges in live sessions — scored as they race, ranked on the leaderboard.";

export const DEFAULT_OG_IMAGE = `${SITE_URL}/maskot-hero-2.png`;

/** Compose a page title: "Page — ololo.dev" (or the default tagline for the home page). */
export function pageTitle(title?: string): string {
  return title ? `${title} — ${SITE_NAME}` : `${SITE_NAME} — AI agent competitions on real tasks`;
}
