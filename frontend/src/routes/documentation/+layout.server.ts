import yaml from "js-yaml";
import type { LayoutServerLoad } from "./$types";

// _nav.yml is inlined as a string by Vite at build time — no runtime fs access needed.
// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore — Vite ?raw suffix types are declared in vite/client
import navRaw from "../../content/_nav.yml?raw";

interface NavEntry {
  id: string;
  label: string;
}

interface DocItem {
  slug: string;
  title: string;
}

interface Section {
  id: string;
  label: string;
  items: DocItem[];
}

const nav = yaml.load(navRaw as string) as NavEntry[];

// Eagerly import all .md files so we can read their frontmatter metadata at load time.
// The glob path is an absolute Vite path (relative to project root = frontend/).
const allFiles = import.meta.glob("/src/content/**/*.md", { eager: true });

/** Strip leading "01-" style prefix from a filename and remove the extension. */
function fileToSlug(filename: string): string {
  return filename.replace(/^\d+-/, "").replace(/\.md$/, "");
}

const sections: Section[] = nav.map((entry) => {
  const prefix = `/src/content/${entry.id}/`;

  const items: (DocItem & { order: number })[] = Object.entries(allFiles)
    .filter(([path]) => path.startsWith(prefix))
    .map(([path, mod]) => {
      const filename = path.slice(prefix.length); // e.g. "01-overview.md"
      const slug = fileToSlug(filename);
      const order = parseInt(filename.match(/^(\d+)/)?.[1] ?? "99", 10);
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const title = ((mod as any).metadata?.title as string) ?? slug;
      return { slug, title, order };
    })
    .sort((a, b) => a.order - b.order);

  return {
    id: entry.id,
    label: entry.label,
    items: items.map(({ slug, title }) => ({ slug, title })),
  };
});

interface FlatItem {
  slug: string;
  title: string;
  sectionLabel: string;
}

const flatItems: FlatItem[] = sections.flatMap((s) =>
  s.items.map((item) => ({ slug: item.slug, title: item.title, sectionLabel: s.label })),
);

export const load: LayoutServerLoad = () => ({ sections, flatItems });
