/**
 * Syntax highlighting for the player Files view, on shiki.
 *
 * Everything here loads on first use: the core, the theme and each grammar
 * are dynamic imports, so no page pays for highlighting until a file is
 * opened, and a Rust repo never downloads the Python grammar. The JavaScript
 * regex engine stands in for oniguruma, which would add a wasm download.
 */
import type { HighlighterCore } from "shiki/core";

const THEME = "github-light";

/**
 * Past this the tokenizer blocks the main thread for noticeable time;
 * such files stay plain text (they are rare in a session repo).
 */
export const MAX_HIGHLIGHT_BYTES = 200_000;

/** Extensions shiki does not list as a language alias. */
const EXTRA_EXT: Record<string, string> = {
  h: "c",
  hh: "cpp",
  hpp: "cpp",
  hxx: "cpp",
  ex: "elixir",
  exs: "elixir",
};

/** Whole file names that carry the language instead of an extension. */
const BY_NAME: Record<string, string> = {
  dockerfile: "dockerfile",
  makefile: "makefile",
  justfile: "just",
  gemfile: "ruby",
  rakefile: "ruby",
};

let languageIds: Promise<Set<string>> | null = null;
let highlighter: Promise<HighlighterCore> | null = null;

function knownLanguages(): Promise<Set<string>> {
  languageIds ??= import("shiki/langs").then((m) => new Set(Object.keys(m.bundledLanguages)));
  return languageIds;
}

/** The shiki language for a repo path, or `null` to show it as plain text. */
export async function languageFor(path: string): Promise<string | null> {
  const name = path.slice(path.lastIndexOf("/") + 1).toLowerCase();
  if (name in BY_NAME) return BY_NAME[name];
  if (name.startsWith("dockerfile.")) return "dockerfile";
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return null;
  const ext = name.slice(dot + 1);
  const lang = EXTRA_EXT[ext] ?? ext;
  return (await knownLanguages()).has(lang) ? lang : null;
}

function loadHighlighter(): Promise<HighlighterCore> {
  highlighter ??= (async () => {
    const [{ createHighlighterCore }, { createJavaScriptRegexEngine }, { bundledThemes }] =
      await Promise.all([
        import("shiki/core"),
        import("shiki/engine/javascript"),
        import("shiki/themes"),
      ]);
    return createHighlighterCore({
      engine: createJavaScriptRegexEngine({ forgiving: true }),
      themes: [bundledThemes[THEME]()],
      langs: [],
    });
  })();
  // A failed load (offline, a bad chunk) must not poison every later file.
  highlighter.catch(() => (highlighter = null));
  return highlighter;
}

/**
 * Highlighted HTML for `code` — coloured `<span>`s separated by `<br>`,
 * meant to sit inside an existing `<pre>` — or `null` when the file is not
 * worth highlighting or highlighting fails (the caller keeps plain text).
 * shiki escapes the source, so the result is safe for `{@html}`.
 */
export async function highlightToHtml(code: string, path: string): Promise<string | null> {
  if (code.length > MAX_HIGHLIGHT_BYTES) return null;
  try {
    const lang = await languageFor(path);
    if (!lang) return null;
    const hl = await loadHighlighter();
    if (!hl.getLoadedLanguages().includes(lang)) {
      const { bundledLanguages } = await import("shiki/langs");
      await hl.loadLanguage(bundledLanguages[lang as keyof typeof bundledLanguages]);
    }
    return hl.codeToHtml(code, { lang, theme: THEME, structure: "inline" });
  } catch {
    return null;
  }
}
