import type { PlayerProbeEntry, PlayerTaskSummaryEntry } from "$lib/types/arena";

/**
 * The two values a player argues with. Some probe types keep the expected
 * answer secret, so it can legitimately be absent while the answer is not.
 */
export function expectedOf(probe: PlayerProbeEntry): string | null {
  return probe.result?.expected ?? probe.expected ?? probe.expected_answer ?? null;
}

export function actualOf(probe: PlayerProbeEntry): string | null {
  return probe.result?.actual ?? probe.actual ?? null;
}

/** The probe's verdict for a screen: unresolved probes are still "pending". */
export function probeStatus(p: PlayerProbeEntry): "pass" | "fail" | "pending" {
  if (p.state !== "resolved" && !p.result) return "pending";
  const pass =
    p.outcome === "pass" || p.result?.status === "passed" || p.result?.status === "correct";
  return pass ? "pass" : "fail";
}

/** For a screen: an absent value reads as the caller's word, an empty one as itself. */
export function shownValue(value: string | null, absent: string): string {
  if (value === null) return absent;
  return value === "" ? "(empty)" : value;
}

/** Same, for markdown — an empty code span reads as a formatting slip. */
function asCode(value: string | null, absent: string): string {
  if (value === null) return absent;
  return value === "" ? "an empty answer" : `\`${value}\``;
}

/**
 * A task and one of its probes as a briefing to paste into a coding agent:
 * the requirement, the verdict to overturn, and the exact probe that produced
 * it.
 *
 * Markdown, because that is what every agent chat reads best, and because the
 * task text is already authored in it. Everything the player can see goes in —
 * retyping the failing command by hand is precisely the busywork the copy
 * button exists to remove.
 */
export function probeBriefing(
  task: Pick<PlayerTaskSummaryEntry, "ordinal" | "title" | "content">,
  probe: PlayerProbeEntry | null,
  verdictLabel: string,
): string {
  const out = [`## Task ${task.ordinal}: ${task.title}`, "", task.content.trim(), ""];
  if (!probe) {
    out.push("No probe has run for this task yet.");
    return out.join("\n");
  }
  out.push(`### Probe: ${verdictLabel}`, "");
  // Both lines always, even when the probe passed: an agent asked to keep a
  // green task green needs to know what "right" looked like.
  out.push(`- Expected: ${asCode(expectedOf(probe), "hidden for this probe type")}`);
  out.push(`- Got: ${asCode(actualOf(probe), "no answer")}`);
  if (probe.point_delta !== 0) {
    out.push(`- Points: ${probe.point_delta > 0 ? "+" : ""}${probe.point_delta}`);
  }
  if (probe.exit_code !== null) out.push(`- Exit code: ${probe.exit_code}`);
  if (probe.duration_ms !== null) out.push(`- Duration: ${probe.duration_ms}ms`);
  out.push("", "The probe that was run:", "", "```sh", probe.rendered_command.trim(), "```");
  if (probe.fixture_values) {
    out.push("", "Rendered with these fixture values:", "", "```json", probe.fixture_values, "```");
  }
  if (probe.outcome) {
    out.push("", "What the probe printed:", "", "```", probe.outcome.trim(), "```");
  }
  return out.join("\n");
}
