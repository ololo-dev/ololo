import type { RequestHandler } from "./$types";
import { listProjects } from "$lib/api";
import { SITE_URL } from "$lib/seo";

// Documentation slugs, derived from the same content glob the docs layout uses.
const docFiles = import.meta.glob("/src/content/**/*.md");

function docSlugs(): string[] {
  return Object.keys(docFiles).map((path) => {
    const filename = path.split("/").pop() ?? "";
    return filename.replace(/^\d+-/, "").replace(/\.md$/, "");
  });
}

function url(loc: string, lastmod?: string): string {
  const lastmodTag = lastmod ? `<lastmod>${lastmod.slice(0, 10)}</lastmod>` : "";
  return `<url><loc>${loc}</loc>${lastmodTag}</url>`;
}

export const GET: RequestHandler = async ({ fetch, setHeaders }) => {
  // Unauthenticated call — the backend returns public projects only.
  const projects = await listProjects(false, { fetch }).catch(() => []);

  const entries = [
    url(`${SITE_URL}/`),
    url(`${SITE_URL}/projects`),
    ...docSlugs().map((slug) => url(`${SITE_URL}/documentation/${slug}`)),
    ...projects
      .filter((p) => p.slug !== null && !p.archived_at)
      .map((p) => url(`${SITE_URL}/projects/${p.slug}`, p.updated_at)),
  ];

  const body = `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
${entries.join("\n")}
</urlset>
`;

  setHeaders({
    "Content-Type": "application/xml",
    "Cache-Control": "public, max-age=3600",
  });
  return new Response(body);
};
