import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { notify } from "./notifications.svelte";

describe("notify store", () => {
  beforeEach(() => {
    notify.dismissAll();
  });

  afterEach(() => {
    vi.useRealTimers();
    notify.dismissAll();
  });

  it("exposes an empty list initially", () => {
    notify.dismissAll();
    expect(notify.items).toEqual([]);
  });

  it("add pushes a notification with a generated id and default duration", () => {
    const id = notify.add({ type: "info", message: "hi" });
    expect(notify.items).toHaveLength(1);
    expect(notify.items[0]).toMatchObject({
      id,
      type: "info",
      message: "hi",
      duration: 5000,
    });
  });

  it("keeps distinct notifications until dismissed", () => {
    notify.add({ type: "success", message: "a" });
    notify.add({ type: "error", message: "b" });
    expect(notify.items).toHaveLength(2);
  });

  it("success/error/warning/info helpers set the correct type", () => {
    notify.success("ok");
    notify.error("bad");
    notify.warning("careful");
    notify.info("psst");
    expect(notify.items.map((n) => n.type)).toEqual(["success", "error", "warning", "info"]);
  });

  it("dismiss removes only the matching notification by id", () => {
    const a = notify.add({ type: "info", message: "a" });
    const b = notify.add({ type: "info", message: "b" });
    notify.dismiss(a);
    expect(notify.items.map((n) => n.id)).toEqual([b]);
  });

  it("dismissAll clears every notification", () => {
    notify.success("a");
    notify.success("b");
    notify.dismissAll();
    expect(notify.items).toEqual([]);
  });

  it("auto-dismisses a notification after its duration elapses", () => {
    vi.useFakeTimers();
    notify.add({ type: "info", message: "bye", duration: 1000 });
    expect(notify.items).toHaveLength(1);
    vi.advanceTimersByTime(1000);
    expect(notify.items).toEqual([]);
  });

  it("does not auto-dismiss persistent notifications (duration 0)", () => {
    vi.useFakeTimers();
    notify.add({ type: "error", message: "stay", duration: 0 });
    vi.advanceTimersByTime(10000);
    expect(notify.items).toHaveLength(1);
  });
});
