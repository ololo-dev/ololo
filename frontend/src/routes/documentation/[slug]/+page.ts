import { error, redirect } from "@sveltejs/kit";
import type { PageLoad } from "./$types";

// Lazy glob — each entry is a () => Promise<module> loader.
// Built once at module level so Vite can statically analyse the pattern.
const loaders = import.meta.glob("/src/content/**/*.md");

/** Strip leading numeric prefix (e.g. "01-") and ".md" extension → slug. */
function pathToSlug(path: string): string {
  const filename = path.split("/").at(-1) ?? "";
  return filename.replace(/^\d+-/, "").replace(/\.md$/, "");
}

// slug → loader map (built once)
const slugMap: Record<string, () => Promise<unknown>> = {};
for (const [path, loader] of Object.entries(loaders)) {
  slugMap[pathToSlug(path)] = loader;
}

// Renamed pages: old slug → current slug, so published links keep working.
const slugRedirects: Record<string, string> = {
  leaderboard: "arena",
};

export const load: PageLoad = async ({ params }) => {
  const target = slugRedirects[params.slug];
  if (target) redirect(308, `/documentation/${target}`);
  const loader = slugMap[params.slug];
  if (!loader) error(404, `Documentation page "${params.slug}" not found`);

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const mod = (await loader()) as any;
  return {
    // The default export is the mdsvex Svelte component — non-serializable,
    // so this is a universal (not server) load; SvelteKit re-runs on the client.
    component: mod.default,
    metadata: (mod.metadata ?? {}) as Record<string, string>,
  };
};
