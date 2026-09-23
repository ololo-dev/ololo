/**
 * When a judge's artifact request closes. The game server stamps every
 * dispatch of the request with `# Open for <N>s more.` — the time the
 * request stays open from that moment (its phase cap, counted from the
 * registration or the latest delivery). Added to the dispatch time, that is
 * the deadline the reader counts down to.
 */
const OPEN_RE = /^# Open for (\d+)s more\.$/m;

/** Epoch ms the request closes, or null when the command carries no stamp. */
export function requestOpenUntil(
  renderedCommand: string | null | undefined,
  dispatchedAt: string | null | undefined,
): number | null {
  if (!renderedCommand || !dispatchedAt) return null;
  const m = OPEN_RE.exec(renderedCommand);
  if (!m) return null;
  const at = Date.parse(dispatchedAt);
  if (Number.isNaN(at)) return null;
  return at + Number(m[1]) * 1000;
}

/** `4:07` — minutes and seconds left, floored at zero. */
export function formatCountdown(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}
