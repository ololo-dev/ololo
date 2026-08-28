// Shared session-status display maps (badge label + badge classes).

export const statusLabels: Record<string, string> = {
  lobby: "In Lobby",
  running: "Running",
  paused: "Paused",
  finished: "Finished",
  cancelled: "Cancelled",
};

/**
 * What `/s/<code>` will actually show, so a link to it can say so.
 *
 * That one URL is three different pages depending on where the session is:
 * it redirects to the lobby, or shows the live board, or shows the finished
 * report. Links to it used to be labelled with the six-character join code,
 * which told a reader neither where they were going nor that it was a link
 * to a session at all.
 */
export function sessionLinkLabel(status: string): string {
  switch (status) {
    case "finished":
    case "cancelled":
      return "Session results";
    case "lobby":
      return "Session lobby";
    default:
      return "Live board"; // running / paused / unknown
  }
}

/**
 * The same destination in one word, for a column that repeats it on every row.
 *
 * Pair it with [`sessionLinkLabel`] as the link's accessible name: the full
 * phrase is what a screen reader should announce, and eighteen characters per
 * row is what pushed the profile's session table past its container until the
 * column was clipped off the page.
 */
export function sessionLinkShort(status: string): string {
  switch (status) {
    case "finished":
    case "cancelled":
      return "Results";
    case "lobby":
      return "Lobby";
    default:
      return "Live";
  }
}

export function statusClass(status: string): string {
  switch (status) {
    case "running":
      return "bg-green-100 text-green-700";
    case "finished":
      return "bg-gray-100 text-gray-500";
    case "cancelled":
      return "bg-red-100 text-red-600";
    default:
      return "bg-amber-100 text-amber-700"; // lobby / unknown
  }
}
