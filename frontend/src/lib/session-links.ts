/**
 * Linked sessions on the board: the slider's stops.
 *
 * A host runs a few sessions as one event; the board flips between them,
 * and once every one has ended, to the combined standings. These are the
 * pure parts — what the stops are and where each leads — so the page and
 * the screen agree, and a test can read them without a DOM.
 */
import type { SessionLinks } from "$lib/api";

/** Which board the screen shows: this session's, or the group's combined standings. */
export type ScreenSlide = "session" | "overall";

/** Where a slider stop leads: another linked session (by join code), or the
 * combined standings. */
export type SlideTarget = { code: string } | "overall";

/** What a session is called on the board and in the card: its own name, or
 * its join code. The group runs one project, so the project's name would
 * say the same of every room — and a room created from the web is named
 * after its project, which counts as no name of its own. When another
 * session in the group goes by the same own name, the join code follows
 * it, so the two are told apart. */
export function sessionLabel(
  sessions: { join_code: string; name: string; project_name?: string | null }[],
  index: number,
): string {
  const own = (x: (typeof sessions)[number]) => (x.name !== x.project_name ? x.name : "");
  const s = sessions[index];
  const name = own(s);
  if (!name) return s.join_code;
  const twin = sessions.some((x, i) => i !== index && own(x) === name);
  return twin ? `${name} · ${s.join_code}` : name;
}

export interface SlideStop {
  /** The join code, or "overall". */
  key: string;
  label: string;
  target: SlideTarget;
  /** The stop the board is on now. */
  current: boolean;
}

/**
 * One stop per linked session, in the order they were linked, and a last
 * stop for the combined standings — live while the rooms play (they run
 * side by side), final once every one of them has ended. Empty for a
 * session linked to nothing (or a group of one): no slider.
 */
export function slideStops(links: SessionLinks | null, slide: ScreenSlide): SlideStop[] {
  if (!links || links.sessions.length < 2) return [];
  const out: SlideStop[] = links.sessions.map((s, i) => ({
    key: s.join_code,
    label: sessionLabel(links.sessions, i),
    target: { code: s.join_code },
    current: s.current && slide === "session",
  }));
  out.push({ key: "overall", label: "Overall", target: "overall", current: slide === "overall" });
  return out;
}

/** The stop `step` places from the current one, wrapping round; null when
 * there is nowhere to go (no stops, or the one stop is the current one). */
export function stopBy(stops: SlideStop[], step: number): SlideStop | null {
  if (stops.length === 0) return null;
  const active = stops.findIndex((s) => s.current);
  const from = active < 0 ? 0 : active;
  const next = stops[(((from + step) % stops.length) + stops.length) % stops.length];
  return next && !next.current ? next : null;
}

/** Read the slide off a page URL: `?slide=overall`, or the session's own board. */
export function slideFromUrl(url: URL): ScreenSlide {
  return url.searchParams.get("slide") === "overall" ? "overall" : "session";
}
