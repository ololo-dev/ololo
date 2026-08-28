import "@testing-library/jest-dom/vitest";
import { afterAll, afterEach, vi } from "vitest";

// Let timers scheduled during unmount run while the DOM globals still exist.
//
// vaul-svelte restores the body position on unmount via
// `setTimeout(() => requestAnimationFrame(...))`. Testing-library unmounts in
// its own afterEach, so that timer is still pending when the run ends; it then
// fires in bare Node, after vitest has torn the jsdom environment down and
// removed `requestAnimationFrame` with it. The bare identifier no longer
// resolves, and the ReferenceError fails the whole run as an Uncaught
// Exception — with every test passing. (A polyfill cannot help: jsdom already
// provides rAF here because vitest defaults `pretendToBeVisual`, so it is
// jsdom's own global that teardown deletes.) Yielding after each test lets the
// callback run inside the environment instead — two macrotasks, because the
// chain is itself two tasks long: the first yield runs vaul's setTimeout,
// which only then schedules the rAF the second yield lets fire.
afterEach(async () => {
  await new Promise((resolve) => setTimeout(resolve, 0));
  await new Promise((resolve) => setTimeout(resolve, 0));
});

// Belt to the braces above: the per-test yields lose the race when an
// unmount hook registered before ours runs after them (hook order is LIFO,
// so that depends on import order inside each test file). One real wait at
// file end drains any straggler timer chain while the environment is still
// up — CI hit exactly this on a run where every test passed
// (ReferenceError: requestAnimationFrame is not defined, run 31793343392).
afterAll(async () => {
  await new Promise((resolve) => setTimeout(resolve, 25));
});

// jsdom does not ship ResizeObserver, so we provide a no-op stub to prevent
// "ReferenceError: ResizeObserver is not defined" in tests.
if (typeof ResizeObserver === "undefined") {
  global.ResizeObserver = class ResizeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

// jsdom (without pretendToBeVisual) ships no requestAnimationFrame. vaul-svelte
// schedules one from a setTimeout while restoring body position, and the timer
// firing without rAF crashes the whole vitest run as an Uncaught Exception even
// when every test passed (flaky: depends on whether the timer wins the race
// against worker shutdown).
if (typeof globalThis.requestAnimationFrame === "undefined") {
  globalThis.requestAnimationFrame = (cb: FrameRequestCallback): number =>
    setTimeout(() => cb(performance.now()), 0) as unknown as number;
  globalThis.cancelAnimationFrame = (id: number) => clearTimeout(id);
}

// jsdom has no matchMedia; uPlot probes it on load and floods stderr with
// "matchMedia is not a function" in the chart tests.
if (typeof window !== "undefined" && typeof window.matchMedia === "undefined") {
  Object.defineProperty(window, "matchMedia", {
    writable: true,
    value: (query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }),
  });
}

// jsdom does not implement elem.scroll() / elem.scrollTo() on HTMLElement, so
// we add no-op stubs to prevent "not a function" errors.
if (typeof Element !== "undefined") {
  if (!Element.prototype.scroll) {
    Element.prototype.scroll = function () {};
  }
  if (!Element.prototype.scrollTo) {
    Element.prototype.scrollTo = function () {};
  }
}

if (typeof window !== "undefined") {
  Object.defineProperty(window, "scrollTo", {
    writable: true,
    value: () => {},
  });
}

if (typeof globalThis.localStorage === "undefined") {
  const store = new Map<string, string>();
  globalThis.localStorage = {
    getItem: (key: string) => store.get(key) ?? null,
    setItem: (key: string, value: string) => {
      store.set(key, value);
    },
    removeItem: (key: string) => {
      store.delete(key);
    },
    clear: () => {
      store.clear();
    },
    key: (index: number) => Array.from(store.keys())[index] ?? null,
    get length() {
      return store.size;
    },
  } as Storage;
}

vi.mock("ws", () => ({
  default: class MockWs {
    constructor(_url: string) {}
    close() {}
  },
}));

if (typeof window !== "undefined") {
  class MockWebSocket {
    static CONNECTING = 0;
    static OPEN = 1;
    static CLOSING = 2;
    static CLOSED = 3;
    readyState = MockWebSocket.OPEN;
    onopen: ((event: Event) => void) | null = null;
    onmessage: ((event: MessageEvent) => void) | null = null;
    onclose: ((event: CloseEvent) => void) | null = null;
    onerror: ((event: Event) => void) | null = null;
    constructor(_url: string) {}
    close() {
      this.readyState = MockWebSocket.CLOSED;
      this.onclose?.({} as CloseEvent);
    }
    send(_data: string) {}
  }
  Object.defineProperty(window, "WebSocket", {
    writable: true,
    value: MockWebSocket,
  });
}

// vaul-svelte schedules `setTimeout(() => requestAnimationFrame(...))` from
// its position-fixed helper; when the timer fires after a test's jsdom window
// is torn down, the bare `requestAnimationFrame` global is gone and vitest
// fails the whole run with an unhandled ReferenceError (flaky on CI). Pin a
// process-global fallback so the stray callback is harmless.
globalThis.requestAnimationFrame ??= ((cb: FrameRequestCallback) =>
  setTimeout(() => cb(performance.now()), 0)) as unknown as typeof requestAnimationFrame;
globalThis.cancelAnimationFrame ??= ((id: number) =>
  clearTimeout(id)) as unknown as typeof cancelAnimationFrame;
