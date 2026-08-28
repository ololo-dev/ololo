import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const env = vi.hoisted(() => ({ browser: true }));

vi.mock("$app/environment", () => ({
  get browser() {
    return env.browser;
  },
  building: false,
  dev: false,
  version: "test",
}));

import { getToken, setToken, clearToken, silentRefresh, initAuth } from "./auth";

const TOKEN_KEY = "arena_access_token";

function makeJwt(expSeconds: number): string {
  const header = btoa(JSON.stringify({ alg: "HS256", typ: "JWT" }));
  const payload = btoa(JSON.stringify({ exp: expSeconds }));
  return `${header}.${payload}.sig`;
}

const futureJwt = () => makeJwt(Math.floor(Date.now() / 1000) + 3600);

describe("auth token storage", () => {
  let fetchMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    env.browser = true;
    localStorage.clear();
    clearToken();
    vi.useFakeTimers();
    fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    clearToken();
    localStorage.clear();
  });

  it("getToken returns null before any token is set", () => {
    expect(getToken()).toBeNull();
  });

  it("setToken stores the token in memory", () => {
    const jwt = futureJwt();
    setToken(jwt);
    expect(getToken()).toBe(jwt);
  });

  it("setToken persists the token to localStorage when in the browser", () => {
    const jwt = futureJwt();
    setToken(jwt);
    expect(localStorage.getItem(TOKEN_KEY)).toBe(jwt);
  });

  it("setToken does not touch localStorage when not in the browser", () => {
    env.browser = false;
    setToken(futureJwt());
    expect(localStorage.getItem(TOKEN_KEY)).toBeNull();
  });

  it("clearToken wipes the token from memory and localStorage", () => {
    const jwt = futureJwt();
    setToken(jwt);
    clearToken();
    expect(getToken()).toBeNull();
    expect(localStorage.getItem(TOKEN_KEY)).toBeNull();
  });
});

describe("silentRefresh", () => {
  beforeEach(() => {
    env.browser = true;
    localStorage.clear();
    clearToken();
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("returns false without calling fetch when not in the browser", async () => {
    env.browser = false;
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    const ok = await silentRefresh();
    expect(ok).toBe(false);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("stores the returned access_token and returns true on success", async () => {
    const jwt = futureJwt();
    const fetchMock = vi.fn(async () => ({
      ok: true,
      json: async () => ({ access_token: jwt }),
    }));
    vi.stubGlobal("fetch", fetchMock);
    const ok = await silentRefresh();
    expect(ok).toBe(true);
    expect(getToken()).toBe(jwt);
  });

  it("clears the token and returns false when the refresh endpoint is not ok", async () => {
    setToken(futureJwt());
    const fetchMock = vi.fn(async () => ({ ok: false, json: async () => ({}) }));
    vi.stubGlobal("fetch", fetchMock);
    const ok = await silentRefresh();
    expect(ok).toBe(false);
    expect(getToken()).toBeNull();
  });

  it("returns false on network error without clearing an existing token", async () => {
    const jwt = futureJwt();
    setToken(jwt);
    const fetchMock = vi.fn(async () => {
      throw new Error("network down");
    });
    vi.stubGlobal("fetch", fetchMock);
    const ok = await silentRefresh();
    expect(ok).toBe(false);
    expect(getToken()).toBe(jwt);
  });
});

describe("initAuth", () => {
  beforeEach(() => {
    env.browser = true;
    localStorage.clear();
    clearToken();
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("returns early without setting a token when not in the browser", async () => {
    env.browser = false;
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    await initAuth();
    expect(getToken()).toBeNull();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("uses a still-valid token from localStorage without calling refresh", async () => {
    const jwt = futureJwt();
    localStorage.setItem(TOKEN_KEY, jwt);
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    await initAuth();
    expect(getToken()).toBe(jwt);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("performs a silent refresh when no token is present", async () => {
    const jwt = futureJwt();
    const fetchMock = vi.fn(async () => ({
      ok: true,
      json: async () => ({ access_token: jwt }),
    }));
    vi.stubGlobal("fetch", fetchMock);
    await initAuth();
    expect(fetchMock).toHaveBeenCalledWith(
      "/auth/refresh",
      expect.objectContaining({ method: "POST" }),
    );
    expect(getToken()).toBe(jwt);
  });
});
