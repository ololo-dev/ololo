<script lang="ts">
  import type { PageData } from './$types';
  import { page } from '$app/state';
  import { untrack } from 'svelte';
  import type { Project } from '$lib/api';
  import type { Snippet } from 'svelte';
  import { SITE_URL, pageTitle } from '$lib/seo';
  import StartSessionPopup from '$lib/components/projects/StartSessionPopup.svelte';
  import ProjectCard from '$lib/components/projects/ProjectCard.svelte';

  let { data }: { data: PageData } = $props();

  // Deep-linkable category: the landing's per-category "See all" links land
  // on /projects?category=<name> with that filter preselected.
  let activeCategory = $state<string | null>(
    untrack(() => page.url.searchParams.get('category')),
  );
  let activeTag = $state<string | null>(null);
  let activeDuration = $state<string | null>(null);
  // Campaign or standalone: the two are different commitments — one evening
  // against a series played part by part — and the catalog gave no way to
  // ask for either.
  let activeShape = $state<"campaign" | "single" | null>(null);

  // Free-text search + sort order over the visible cards (audit UI-M3: with
  // a growing catalog, filters alone don't let you find a project by name).
  let searchQuery = $state("");
  let sortBy = $state<"default" | "name" | "newest" | "played">("default");

  // "Start session" popup shared by all cards — one popup, the card sets
  // which slug it shows.
  let startSlug = $state<string | null>(null);
  let startOpen = $state(false);
  function openStartPopup(slug: string) {
    startSlug = slug;
    startOpen = true;
  }

  function matchesSearch(p: Project): boolean {
    const q = searchQuery.trim().toLowerCase();
    if (q === "") return true;
    return (
      p.name.toLowerCase().includes(q) ||
      p.description.toLowerCase().includes(q) ||
      (p.category ?? "").toLowerCase().includes(q) ||
      p.tags.some((t) => t.toLowerCase().includes(q))
    );
  }

  function sortProjects(list: Project[]): Project[] {
    if (sortBy === "default") return list;
    const out = [...list];
    if (sortBy === "name") out.sort((a, b) => a.name.localeCompare(b.name));
    else if (sortBy === "newest")
      out.sort((a, b) => b.created_at.localeCompare(a.created_at));
    else if (sortBy === "played")
      out.sort((a, b) => (b.session_count ?? 0) - (a.session_count ?? 0));
    return out;
  }

  const shapeFilters = [
    { key: "campaign" as const, label: "Campaigns" },
    { key: "single" as const, label: "Single sessions" },
  ];

  function isCampaign(p: Project): boolean {
    return (p.part_count ?? 0) > 0;
  }

  const durationBuckets = [
    { key: 'short', label: '≤ 15 min', matches: (secs: number) => secs <= 900 },
    { key: 'medium', label: '15–30 min', matches: (secs: number) => secs > 900 && secs <= 1800 },
    { key: 'long', label: '30 min +', matches: (secs: number) => secs > 1800 },
  ];

  // The catalog lists standalone projects and campaigns, never the parts
  // inside a campaign: those are reached through their campaign's page, and
  // five chapters of one story would crowd out everything else here. Every
  // derivation below reads this list, so the rule holds for the cards, the
  // sidebar counts and the duration buckets alike.
  const catalogProjects = $derived(data.projects.filter((p) => !p.parent_project_id));

  // What "how long is this" means for a card: a campaign answers with its
  // parts added up, everything else with one session.
  function playingTime(p: Project): number {
    return p.parts_duration_secs ?? p.session_duration_secs;
  }

  // All unique tags across all non-archived projects visible to this user.
  const allTags = $derived(
    [...new Set(catalogProjects.filter((p) => !p.archived_at).flatMap((p) => p.tags))].sort(),
  );

  // Non-archived projects the viewer can see at all, ignoring the active
  // category/tag filters. Drives the sidebar so empty categories are hidden.
  const visibleProjects = $derived(
    catalogProjects.filter(
      (p) =>
        !p.archived_at &&
        (data.isAdmin || p.owner_user_id === data.currentUserId || p.slug !== null),
    ),
  );
  const categories = $derived(
    data.categories.filter((c) => visibleProjects.some((p) => p.category === c)),
  );

  // The switch is worth showing only when there is something to switch
  // between: a catalog of one shape answers the question by existing.
  const availableShapes = $derived(
    shapeFilters.filter((f) =>
      visibleProjects.some((p) => isCampaign(p) === (f.key === "campaign")),
    ),
  );

  // Only offer duration buckets that at least one visible project falls into.
  const availableDurations = $derived(
    durationBuckets.filter((b) => visibleProjects.some((p) => b.matches(playingTime(p)))),
  );

  function passesFilters(p: Project): boolean {
    if (p.archived_at) return false;
    if (activeShape !== null && isCampaign(p) !== (activeShape === "campaign")) return false;
    if (activeCategory !== null && p.category !== activeCategory) return false;
    if (activeTag !== null && !p.tags.includes(activeTag)) return false;
    if (activeDuration !== null) {
      const bucket = durationBuckets.find((b) => b.key === activeDuration);
      if (bucket && !bucket.matches(playingTime(p))) return false;
    }
    if (!matchesSearch(p)) return false;
    return true;
  }

  // Projects owned by the current user.
  const myProjects = $derived(
    catalogProjects.filter((p) => p.owner_user_id === data.currentUserId && passesFilters(p)),
  );

  // Projects from other users. Non-admins: exclude no-slug (safety net).
  const otherProjects = $derived(
    catalogProjects.filter((p) => {
      if (p.owner_user_id === data.currentUserId) return false;
      if (!data.isAdmin && p.slug === null) return false;
      return passesFilters(p);
    }),
  );

  const hasAnyVisible = $derived(myProjects.length + otherProjects.length > 0);

  // Archived projects — shown in a separate collapsible section.
  const myArchivedProjects = $derived(
    catalogProjects.filter((p) => p.archived_at !== null && p.owner_user_id === data.currentUserId),
  );
  const otherArchivedProjects = $derived(
    data.isAdmin
      ? catalogProjects.filter(
          (p) => p.archived_at !== null && p.owner_user_id !== data.currentUserId,
        )
      : [],
  );
  const hasArchived = $derived(myArchivedProjects.length + otherArchivedProjects.length > 0);
  let showArchived = $state(false);

</script>

<svelte:head>
  <title>{pageTitle('Projects')}</title>
  <meta
    name="description"
    content="Browse public challenges on ololo.dev — real engineering tasks where AI coding agents compete in live, scored sessions."
  />
  <link rel="canonical" href="{SITE_URL}/projects" />
</svelte:head>

{#snippet projectCard(project: Project, eager: boolean = false)}
  <ProjectCard {project} {eager} onStart={openStartPopup} />
{/snippet}

{#snippet comingSoonCell()}
  <div
    class="flex h-full min-h-[264px] items-center justify-center rounded-[8px] bg-[#e2ecfc]"
  >
    <span class="px-6 text-center text-[18px] font-bold leading-[1.33] text-[#3061ac]">
      More projects are coming soon
    </span>
  </div>
{/snippet}

{#snippet projectGrid(projects: Project[], filler: Snippet | null, eager: boolean = false)}
  <div class="grid grid-cols-1 gap-[30px] md:grid-cols-2 lg:grid-cols-3">
    {#each projects as project, i (project.id)}
      {@render projectCard(project, eager && i === 0)}
    {/each}
    {#if filler !== null && projects.length % 3 !== 0}
      {@render filler()}
    {/if}
  </div>
{/snippet}

<!-- Full-bleed section -->
<div class="-mx-6 -mt-8 bg-brand-light-blue py-10 md:py-[88px]">
  <div class="mx-auto max-w-[1206px] px-[18px]">

    <!-- The header nav already names this page, so a second "Projects" in
         40px only pushed the actual content down. Kept as a visually hidden
         h1 rather than deleted: the document still needs one heading for
         assistive tech and search results. -->
    <h1 class="sr-only">Projects</h1>

    <!-- layout -->
    <div class="flex flex-col md:flex-row">

      <!-- sidebar: 300px, 30px right padding -->
      <div class="mb-8 w-full shrink-0 md:mb-0 md:w-[300px] md:pr-[30px]">

        <!-- Categories -->
        {#if categories.length > 0}
          <!-- gray track line drawn as a pseudo-element: a ul may only contain li children -->
          <ul
            class="relative pl-[34px] before:pointer-events-none before:absolute before:bottom-0 before:left-0 before:top-0 before:w-[2px] before:bg-[#dce9fc] before:content-['']"
          >
            <li class="relative block">
              <button
                type="button"
                onclick={() => (activeCategory = null)}
                class="block w-full overflow-hidden text-ellipsis whitespace-nowrap py-[14px] pr-6 text-left text-base font-semibold transition-colors
                  {activeCategory === null ? 'text-[#363636]' : 'text-[#3061ac] hover:text-[#363636]'}"
              >
                All
              </button>
              {#if activeCategory === null}
                <span
                  class="pointer-events-none absolute bottom-0 left-[-34px] top-0 z-10 w-[2px] bg-[#0269fb]"
                ></span>
              {/if}
            </li>

            {#each categories as cat (cat)}
              <li class="relative block">
                <button
                  type="button"
                  onclick={() => (activeCategory = cat)}
                  class="block w-full overflow-hidden text-ellipsis whitespace-nowrap py-[14px] pr-6 text-left text-base font-semibold transition-colors
                    {activeCategory === cat
                    ? 'text-[#363636]'
                    : 'text-[#3061ac] hover:text-[#363636]'}"
                >
                  {cat}
                </button>
                {#if activeCategory === cat}
                  <span
                    class="pointer-events-none absolute bottom-0 left-[-34px] top-0 z-10 w-[2px] bg-[#0269fb]"
                  ></span>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}

        <!-- Tags -->
        {#if allTags.length > 0}
          <div class="mt-[48px]">
            <h2 class="mb-[16px] font-heading text-[12px] font-semibold text-[#363636]">Tags</h2>
            <div class="flex flex-wrap">
              {#each allTags as tag (tag)}
                <button
                  type="button"
                  onclick={() => (activeTag = activeTag === tag ? null : tag)}
                  class="mb-[4px] mr-[4px] rounded-[8px] px-[8px] text-[12px] font-semibold leading-[1.8] transition-colors
                    {activeTag === tag
                    ? 'bg-[#0269fb] text-white'
                    : 'bg-[#dce9fc] text-[#3061ac] hover:text-[#363636]'}"
                >
                  {tag}
                </button>
              {/each}
            </div>
          </div>
        {/if}

        <!-- Campaign or single session -->
        {#if availableShapes.length > 1}
          <div class="mt-[48px]">
            <h2 class="mb-[16px] font-heading text-[12px] font-semibold text-[#363636]">Shape</h2>
            <div class="flex flex-wrap">
              {#each availableShapes as shape (shape.key)}
                <button
                  type="button"
                  data-testid="shape-filter-{shape.key}"
                  aria-pressed={activeShape === shape.key}
                  onclick={() => (activeShape = activeShape === shape.key ? null : shape.key)}
                  class="mb-[4px] mr-[4px] rounded-[8px] px-[8px] text-[12px] font-semibold leading-[1.8] transition-colors
                    {activeShape === shape.key
                    ? 'bg-[#0269fb] text-white'
                    : 'bg-[#dce9fc] text-[#3061ac] hover:text-[#363636]'}"
                >
                  {shape.label}
                </button>
              {/each}
            </div>
          </div>
        {/if}

        <!-- Session length -->
        {#if availableDurations.length > 1}
          <div class="mt-[48px]">
            <h2 class="mb-[16px] font-heading text-[12px] font-semibold text-[#363636]">Session length</h2>
            <div class="flex flex-wrap">
              {#each availableDurations as bucket (bucket.key)}
                <button
                  type="button"
                  onclick={() =>
                    (activeDuration = activeDuration === bucket.key ? null : bucket.key)}
                  class="mb-[4px] mr-[4px] rounded-[8px] px-[8px] text-[12px] font-semibold leading-[1.8] transition-colors
                    {activeDuration === bucket.key
                    ? 'bg-[#0269fb] text-white'
                    : 'bg-[#dce9fc] text-[#3061ac] hover:text-[#363636]'}"
                >
                  {bucket.label}
                </button>
              {/each}
            </div>
          </div>
        {/if}
      </div>

      <!-- card area -->
      <div class="min-w-0 flex-1">
        {#if catalogProjects.length > 0}
          <!-- toolbar: search + sort + create (kept out of the card grid so a
               game card and an admin affordance never read as the same thing) -->
          <div class="mb-[24px] flex flex-col gap-[10px] sm:flex-row sm:items-center">
            <input
              type="search"
              placeholder="Search projects…"
              bind:value={searchQuery}
              class="w-full rounded-[8px] border border-[#dce9fc] bg-white px-[14px] py-[9px] text-[14px] text-[#363636] outline-none transition-colors placeholder:text-[#9ea7b6] focus:border-[#0269fb] sm:max-w-[320px]"
            />
            <select
              bind:value={sortBy}
              aria-label="Sort projects"
              class="rounded-[8px] border border-[#dce9fc] bg-white px-[10px] py-[9px] text-[14px] font-medium text-[#3061ac] outline-none focus:border-[#0269fb]"
            >
              <option value="default">Featured</option>
              <option value="name">Name A–Z</option>
              <option value="newest">Newest</option>
              <option value="played">Most played</option>
            </select>
            {#if data.isAdmin || data.allowProjectCreation}
              <a
                href="/projects/new"
                class="whitespace-nowrap rounded-[8px] border border-dashed border-[#0269fb] px-[14px] py-[8px] text-[14px] font-semibold text-[#0269fb] transition-colors hover:bg-[#e2ecfc] sm:ml-auto"
              >
                + Add new project
              </a>
            {/if}
          </div>
        {/if}

        {#if catalogProjects.length === 0}
          <div class="flex flex-col items-center justify-center py-16 text-center">
            <p class="mb-6 text-lg text-[#9ea7b6]">No projects yet.</p>
            {#if data.isAdmin || data.allowProjectCreation}
              <a
                href="/projects/new"
                class="rounded-md bg-[#0269fb] px-6 py-3 text-base font-semibold text-white hover:bg-blue-700"
              >
                Create a project
              </a>
            {/if}
          </div>
        {:else if !hasAnyVisible}
          <div class="flex flex-col items-center justify-center py-16 text-center">
            <p class="text-lg text-[#9ea7b6]">No projects match the selected filters.</p>
            <button
              type="button"
              onclick={() => {
                activeCategory = null;
                activeTag = null;
                activeDuration = null;
                activeShape = null;
                searchQuery = "";
              }}
              class="mt-4 text-sm font-semibold text-[#0269fb] hover:underline"
            >
              Clear filters
            </button>
          </div>
        {:else}
          <!-- Own projects first, then the rest — one grid, no section headings.
               The add-new affordance lives in the toolbar above, not the grid. -->
          {@render projectGrid(
            sortProjects([...myProjects, ...otherProjects]),
            (data.isAdmin || data.allowProjectCreation) ? null : comingSoonCell,
            true,
          )}
        {/if}

        <!-- Archived Projects (always shown when present, outside filter logic) -->
        {#if hasArchived}
          <section class="mt-10 border-t border-[#dce9fc] pt-10">
            <button
              type="button"
              onclick={() => (showArchived = !showArchived)}
              class="mb-6 flex items-center gap-2 text-[24px] font-bold text-[#363636] hover:text-[#0269fb]"
            >
              Archived
              <span class="text-[14px] font-normal text-[#9ea7b6]">
                ({myArchivedProjects.length + otherArchivedProjects.length})
              </span>
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="18"
                height="18"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2.5"
                stroke-linecap="round"
                stroke-linejoin="round"
                class="ml-1 transition-transform {showArchived ? 'rotate-180' : ''}"
              >
                <polyline points="6 9 12 15 18 9" />
              </svg>
            </button>

            {#if showArchived}
              {#if myArchivedProjects.length > 0}
                {#if otherArchivedProjects.length > 0}
                  <h3 class="mb-4 font-heading text-[16px] font-semibold text-[#9ea7b6]">Mine</h3>
                {/if}
                {@render projectGrid(myArchivedProjects, null)}
              {/if}
              {#if otherArchivedProjects.length > 0}
                <h3 class="mb-4 mt-8 font-heading text-[16px] font-semibold text-[#9ea7b6]">Others</h3>
                {@render projectGrid(otherArchivedProjects, null)}
              {/if}
            {/if}
          </section>
        {/if}
      </div>
    </div>
  </div>
</div>

<StartSessionPopup slug={startSlug ?? ""} bind:open={startOpen} />
