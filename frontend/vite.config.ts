import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig } from "vitest/config";
import { loadEnv } from "vite";
import { svelteTesting } from "@testing-library/svelte/vite";

export default defineConfig(({ mode }) => {
  // Single source of truth: read the workspace-root `.env` so SERVER_PORT,
  // PUBLIC_WS_URL, ARENA_API_PROXY_TARGET, etc. all flow from one file.
  // Vite's `loadEnv` defaults to the project root (./frontend); we point
  // it at `..` so it picks up the monorepo .env. SvelteKit's own env
  // loader is configured the same way in svelte.config.js.
  // The empty prefix loads ALL keys (PUBLIC_*, ARENA_*, SERVER_PORT, ...).
  const env = loadEnv(mode, "..", "");

  // Backend listens on SERVER_PORT (default 8080 — see .env). Proxy targets
  // can be overridden directly via ARENA_API_PROXY_TARGET / ARENA_WS_PROXY_TARGET.
  // `??` alone is not enough: an env var set to "" (a shell override meaning
  // "unset this") would otherwise win and produce an empty proxy target.
  const cfg = (key: string) => {
    const v = env[key];
    return v !== undefined && v.trim() !== "" ? v.trim() : undefined;
  };

  const serverPort = cfg("SERVER_PORT") ?? "8080";
  const apiTarget = cfg("ARENA_API_PROXY_TARGET") ?? `http://localhost:${serverPort}`;
  // Default the socket to the API host so pointing ARENA_API_PROXY_TARGET at a
  // deployment moves the WebSocket with it (http->ws, https->wss).
  const wsTarget = cfg("ARENA_WS_PROXY_TARGET") ?? apiTarget.replace(/^http/, "ws");
  const devPort = Number(cfg("ARENA_FRONTEND_PORT") ?? "5173");

  /**
   * Is the API somewhere other than this machine? Pointing
   * `ARENA_API_PROXY_TARGET` at e.g. https://plum.ololo.dev lets frontend work
   * run against a deployed backend, which is much faster than rebuilding the
   * Rust server for a CSS change.
   */
  const remoteApi = !/^\w+:\/\/(localhost|127\.0\.0\.1|\[::1\])(:|$)/.test(apiTarget);

  /**
   * Three things have to be rewritten before a browser on localhost can talk
   * to a deployed backend through this proxy. Each is silent-failure-shaped,
   * which is why they are spelled out:
   *
   * 1. `Host` — the deployment sits behind a router that dispatches on it, so
   *    `Host: localhost:5173` 404s. `changeOrigin` fixes this.
   * 2. `Origin` — the server's origin guard checks it against
   *    ARENA_FRONTEND_ORIGINS, which will never list localhost. We present
   *    ourselves as the deployment's own frontend.
   * 3. `Set-Cookie` — auth cookies come back scoped `Domain=<deployment>;
   *    Secure`, and a browser on http://localhost silently drops both. Strip
   *    those attributes so the session actually sticks.
   */
  const proxyOpts = (target: string, ws = false) =>
    remoteApi
      ? {
          target,
          ws,
          changeOrigin: true,
          configure: (proxy: { on: (event: string, cb: (...args: never[]) => void) => void }) => {
            const httpOrigin = apiTarget.replace(/^ws/, "http");
            proxy.on("proxyReq", ((proxyReq: { setHeader: (k: string, v: string) => void }) => {
              proxyReq.setHeader("origin", httpOrigin);
            }) as never);
            proxy.on("proxyRes", ((proxyRes: {
              headers: Record<string, string | string[] | undefined>;
            }) => {
              const cookies = proxyRes.headers["set-cookie"];
              if (Array.isArray(cookies)) {
                proxyRes.headers["set-cookie"] = cookies.map((c) =>
                  c.replace(/;\s*Domain=[^;]*/gi, "").replace(/;\s*Secure/gi, ""),
                );
              }
            }) as never);
          },
        }
      : { target, ws, changeOrigin: false };

  return {
    plugins: [sveltekit(), svelteTesting()],
    // bytemd ships raw Svelte 3 source files under its `svelte` export condition
    // which the Svelte 5 compiler cannot process (uses the reserved `$` name).
    // Force both the browser and SSR bundles to use the pre-compiled dist instead.
    resolve: {
      alias: [
        {
          // Match the bare `bytemd` specifier only — NOT `bytemd/dist/index.css` etc.
          // This bypasses the `svelte` export condition (raw Svelte 3 files) and
          // points straight at the pre-compiled ESM dist that Svelte 5 can handle.
          find: /^bytemd$/,
          replacement: new URL("./node_modules/bytemd/dist/index.mjs", import.meta.url).pathname,
        },
      ],
    },
    ssr: {
      // Belt-and-suspenders: also mark external for SSR so the alias is never
      // overridden by the SvelteKit SSR resolver picking up the svelte condition.
      external: ["bytemd", "@bytemd/plugin-gfm", "@bytemd/plugin-highlight"],
    },
    server: {
      port: devPort,
      allowedHosts: ["ololo.local"],
      proxy: {
        "/auth": proxyOpts(apiTarget),
        "/api": proxyOpts(apiTarget),
        "/git": proxyOpts(apiTarget),
        "/ws": proxyOpts(wsTarget, true),
      },
    },
    test: {
      environment: "jsdom",
      globals: true,
      include: ["src/**/*.{test,spec}.{ts,js}"],
      setupFiles: ["src/test-setup.ts"],
    },
  };
});
