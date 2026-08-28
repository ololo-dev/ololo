import adapter from "@sveltejs/adapter-node";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";
import { mdsvex } from "mdsvex";

/** @type {import('@sveltejs/kit').Config} */
const config = {
  extensions: [".svelte", ".md"],
  preprocess: [
    vitePreprocess(),
    mdsvex({
      extensions: [".md"],
      smartypants: false,
      highlight: {
        // Plain pre/code — article CSS handles the visual style
        highlighter: (code) =>
          `<pre><code>${code.replace(/[{}`]/g, (c) => ({ "{": "&#123;", "}": "&#125;", "`": "&#96;" })[c] ?? c)}</code></pre>`,
      },
    }),
  ],
  kit: {
    adapter: adapter(),
    alias: {
      $lib: "./src/lib",
    },
    // Single source of truth: load .env from the workspace root so
    // PUBLIC_WS_URL etc. come from the same file the backend uses.
    env: {
      dir: "..",
    },
  },
};

export default config;
