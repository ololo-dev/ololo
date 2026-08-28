import { describe, expect, it } from "vitest";
import { createReplay } from "./replay.svelte";

describe("createReplay", () => {
  it("starts disengaged and reveals everything", () => {
    const cleanup = $effect.root(() => {
      const r = createReplay("t-start", () => 100);
      expect(r.engaged).toBe(false);
      expect(r.playing).toBe(false);
      expect(r.t).toBe(0);
      expect(r.total).toBe(100);
      expect(r.speed).toBe(1); // 1× is real time
      // Disengaged: no playhead, so consumers show all content.
      expect(r.revealUntil).toBe(null);
    });
    cleanup();
  });

  it("seek engages, pauses, and clamps to [0, total]", () => {
    const cleanup = $effect.root(() => {
      const r = createReplay("t-seek", () => 100);
      r.seek(30);
      expect(r.engaged).toBe(true);
      expect(r.playing).toBe(false);
      expect(r.t).toBe(30);
      expect(r.revealUntil).toBe(30);

      r.seek(999);
      expect(r.t).toBe(100);
      r.seek(-5);
      expect(r.t).toBe(0);
    });
    cleanup();
  });

  it("toggle from the end restarts at zero", () => {
    const cleanup = $effect.root(() => {
      const r = createReplay("t-toggle", () => 100);
      r.seek(100); // parked at the end
      r.toggle(); // play again → rewinds to the top
      expect(r.t).toBe(0);
      expect(r.playing).toBe(true);
      r.toggle(); // pause
      expect(r.playing).toBe(false);
    });
    cleanup();
  });

  it("setSpeed updates the real-time multiplier", () => {
    const cleanup = $effect.root(() => {
      const r = createReplay("t-speed", () => 100);
      expect(r.speed).toBe(1);
      r.setSpeed(8);
      expect(r.speed).toBe(8);
    });
    cleanup();
  });

  it("total tracks the live getter", () => {
    const cleanup = $effect.root(() => {
      let len = 50;
      const r = createReplay("t-total", () => len);
      expect(r.total).toBe(50);
      len = 200;
      expect(r.total).toBe(200);
    });
    cleanup();
  });

  it("shares state across instances with the same key", () => {
    const cleanup = $effect.root(() => {
      const a = createReplay("shared-1", () => 100);
      const b = createReplay("shared-1", () => 100);
      a.seek(42);
      // b is a second view of the same session — it sees a's playhead.
      expect(b.t).toBe(42);
      expect(b.engaged).toBe(true);
      b.setSpeed(4);
      expect(a.speed).toBe(4);
    });
    cleanup();
  });

  it("keeps distinct keys independent", () => {
    const cleanup = $effect.root(() => {
      const a = createReplay("indep-a", () => 100);
      const b = createReplay("indep-b", () => 100);
      a.seek(10);
      expect(b.t).toBe(0);
    });
    cleanup();
  });
});
