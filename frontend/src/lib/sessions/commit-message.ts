/**
 * The snapshot commit-message format (`arena_core::snapshot_message`):
 * `kind(<task uuid>): <subject>` on the first line, and — since format 1 —
 * a trailer block of `Ololo-*: value` lines after a blank line. Readers
 * here split the two so the UI shows the subject as a title and the
 * trailers as metadata, never as a raw body.
 */

export const OLOLO_TRAILER_RE = /^(Ololo-[A-Za-z-]+):\s*(.*)$/;

export interface ParsedCommitMessage {
  subject: string;
  /** Prose body lines (none for snapshot commits). */
  body: string;
  /** `Ololo-*` trailers in order, keys without the `Ololo-` prefix. */
  trailers: { key: string; value: string }[];
}

export function parseCommitMessage(message: string): ParsedCommitMessage {
  const lines = message.split("\n");
  const subject = lines[0] ?? "";
  const rest = lines.slice(1).join("\n").trim();
  if (rest === "") return { subject, body: "", trailers: [] };
  const paragraphs = rest.split(/\n\s*\n/);
  const last = paragraphs[paragraphs.length - 1].split("\n").map((l) => l.trimEnd());
  const isTrailerBlock =
    last.length > 0 && last.every((l) => /^[A-Za-z0-9-]+:\s/.test(l) || l.trim() === "");
  if (!isTrailerBlock) return { subject, body: rest, trailers: [] };
  const trailers = last
    .map((l) => OLOLO_TRAILER_RE.exec(l))
    .filter((m): m is RegExpExecArray => m !== null)
    .map((m) => ({ key: m[1].slice("Ololo-".length), value: m[2] }));
  const body = paragraphs.slice(0, -1).join("\n\n").trim();
  return { subject, body, trailers };
}

/** The trailers a reader cares about on a history card, in display order. */
export const SHOWN_TRAILERS = ["Task-Title", "Probe-Seq", "Outcome", "Timestamp"] as const;

export function displayTrailers(parsed: ParsedCommitMessage): { key: string; value: string }[] {
  const wanted = new Map(parsed.trailers.map((t) => [t.key, t.value]));
  return SHOWN_TRAILERS.filter((k) => wanted.has(k)).map((k) => ({
    key: k === "Probe-Seq" ? "probe" : k.toLowerCase().replace("-", " "),
    value: k === "Probe-Seq" ? `#${wanted.get(k)}` : (wanted.get(k) as string),
  }));
}
