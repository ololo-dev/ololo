import type { ActivityEvent } from "$lib/types/arena";

/** One activity event as the presentation screen's ticker says it: who,
 * what happened, and the points it moved (null when none). The words are
 * the ActivityLog row's, flattened to a line a room reads from the back. */
export interface TickerLine {
  who: string;
  what: string;
  delta: number | null;
}

export function tickerLine(ev: ActivityEvent): TickerLine {
  const who = ev.player_display_name;
  const task = `Task ${ev.task_ordinal}`;
  switch (ev.kind) {
    case "task_started":
      return {
        who,
        what: ev.task_title ? `started ${task} · ${ev.task_title}` : `started ${task}`,
        delta: null,
      };
    case "task_scored":
      if (ev.judge_name) {
        return { who, what: `evaluated by ${ev.judge_name} on ${task}`, delta: ev.point_delta };
      }
      return { who, what: `implemented ${task}`, delta: ev.point_delta };
    case "similarity": {
      const pct = ev.detail?.duplicated_pct ?? 0;
      if (ev.point_delta != null && ev.point_delta < 0) {
        return { who, what: `copy/paste check: ${pct.toFixed(0)}% matched`, delta: ev.point_delta };
      }
      return { who, what: "copy/paste check: clean", delta: null };
    }
    case "artifact_received":
      return { who, what: `delivered a capture for ${task}`, delta: null };
  }
}

/** The newest `limit` events, newest first — the ticker shows the last few
 * things that happened, not the whole log. */
export function latestEvents(events: ActivityEvent[], limit: number): ActivityEvent[] {
  return [...events]
    .sort((a, b) => Date.parse(b.timestamp) - Date.parse(a.timestamp))
    .slice(0, limit);
}
