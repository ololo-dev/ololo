import { browser } from "$app/environment";
import { untrack } from "svelte";

/**
 * Shared session-replay engine. A playhead sweeps *real* session time —
 * measured in elapsed seconds since the session started — and consumers read
 * `revealUntil` to hide anything past it, so the score chart draws in and the
 * activity feed / chat / details reveal their entries.
 *
 * Two properties matter:
 *  - **Real-time speeds.** 1× plays back at wall-clock speed (one session second
 *    per real second); 2× is twice as fast, etc. A full 1× replay therefore
 *    takes exactly as long as the session did.
 *  - **Shared progress.** State is keyed by session code in a module-level
 *    store, so the dashboard and the player page (chat + details) all drive and
 *    reflect the *same* playhead. Scrub on one, navigate to another, and it
 *    continues from where it was. The mounted page owns the rAF loop; on
 *    navigation the next page's effect picks it up.
 *
 * Both pages express the playhead in the same unit — elapsed seconds from
 * session start — so the shared value means the same thing everywhere.
 */
class ReplayState {
  engaged = $state(false); // flips on first play/seek; until then, show everything
  playing = $state(false);
  t = $state(0); // elapsed seconds since session start
  speed = $state(1); // real-time multiplier
}

const stores = new Map<string, ReplayState>();

function getState(key: string): ReplayState {
  let s = stores.get(key);
  if (!s) {
    s = new ReplayState();
    stores.set(key, s);
  }
  return s;
}

export function createReplay(key: string, getTotal: () => number) {
  const s = getState(key);

  // Advance the playhead while playing, at real-time × speed. Reads the
  // position untracked so a new frame doesn't re-arm the loop; seeking pauses,
  // so nothing mutates the position underneath the loop.
  $effect(() => {
    if (!browser || !s.playing) return;
    let cur = untrack(() => s.t);
    let raf = 0;
    let last = 0;
    const step = (now: number) => {
      if (!last) {
        last = now;
        raf = requestAnimationFrame(step);
        return;
      }
      // speed = session-seconds advanced per real second → 1× is wall-clock.
      cur += (s.speed * (now - last)) / 1000;
      last = now;
      const total = getTotal();
      if (cur >= total) {
        s.t = total;
        s.playing = false;
        return;
      }
      s.t = cur;
      raf = requestAnimationFrame(step);
    };
    raf = requestAnimationFrame(step);
    return () => cancelAnimationFrame(raf);
  });

  return {
    get engaged() {
      return s.engaged;
    },
    get playing() {
      return s.playing;
    },
    get t() {
      return s.t;
    },
    get speed() {
      return s.speed;
    },
    get total() {
      return getTotal();
    },
    /** The playhead while engaged, else `null` (show all). */
    get revealUntil(): number | null {
      return s.engaged ? s.t : null;
    },
    toggle() {
      s.engaged = true;
      if (!s.playing && s.t >= getTotal()) s.t = 0; // restart from the top when at the end
      s.playing = !s.playing;
    },
    seek(v: number) {
      s.engaged = true;
      s.playing = false;
      s.t = Math.max(0, Math.min(v, getTotal()));
    },
    setSpeed(sp: number) {
      s.speed = sp;
    },
  };
}

export type Replay = ReturnType<typeof createReplay>;
