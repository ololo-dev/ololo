// Shared time/date formatting helpers.

/** Compact token/count display: `1.2k`, `3.4M`, `—` for null. */
export function formatTokens(n: number | null | undefined): string {
  if (n == null) return "—";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

/** Compact USD: `$1.23`, `$0.045`, `$0.00012`, `—` for null/undefined. */
export function formatUsd(v: number | null | undefined): string {
  if (v == null) return "—";
  if (v >= 1) return `$${v.toFixed(2)}`;
  if (v >= 0.01) return `$${v.toFixed(3)}`;
  return v > 0 ? `$${v.toFixed(5)}` : "$0";
}

/** `[HH, MM, SS]` zero-padded digit triple for timer displays. */
export function countdownDigits(secs: number): [string, string, string] {
  const h = Math.floor(secs / 3_600);
  const m = Math.floor((secs % 3_600) / 60);
  const s = secs % 60;
  return [String(h).padStart(2, "0"), String(m).padStart(2, "0"), String(s).padStart(2, "0")];
}

/** `HH:MM:SS` for live tickers. */
export function formatHms(secs: number): string {
  return countdownDigits(secs).join(":");
}

/** Human duration like `10 min`, `1 h`, `1 h 30 min` for session length. */
export function formatDuration(secs: number): string {
  const mins = Math.round(secs / 60);
  if (mins < 60) return `${mins} min`;
  const h = Math.floor(mins / 60);
  const rest = mins % 60;
  return rest === 0 ? `${h} h` : `${h} h ${rest} min`;
}

/** `m:ss` for short countdowns (e.g. next probe). */
export function formatCountdown(secs: number): string {
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

/** `Jan 5, 2026` in UTC, locale-independent. */
export function formatDateUTC(iso: string): string {
  const d = new Date(iso);
  const months = [
    "Jan",
    "Feb",
    "Mar",
    "Apr",
    "May",
    "Jun",
    "Jul",
    "Aug",
    "Sep",
    "Oct",
    "Nov",
    "Dec",
  ];
  return `${months[d.getUTCMonth()]} ${d.getUTCDate()}, ${d.getUTCFullYear()}`;
}

/**
 * `14:03 UTC`, locale- and timezone-independent.
 *
 * UTC rather than the viewer's clock on purpose: these strings are rendered
 * on the server too, and a local time would disagree with itself between the
 * server-rendered HTML and the hydrated page.
 */
export function formatTimeUTC(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const hh = String(d.getUTCHours()).padStart(2, "0");
  const mm = String(d.getUTCMinutes()).padStart(2, "0");
  return `${hh}:${mm} UTC`;
}

/** UTC date and time, e.g. `Jan 5, 2026, 14:03 UTC`. */
export function formatDateTimeUTC(iso: string): string {
  const time = formatTimeUTC(iso);
  return time ? `${formatDateUTC(iso)}, ${time}` : "";
}

/** Locale short date, e.g. `Jan 5, 2026` in the viewer's locale. */
export function formatDate(iso: string): string {
  return new Date(iso).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}
