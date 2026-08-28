<script lang="ts">
  import type { PageData } from './$types'
  import CodeBlock from '$lib/components/CodeBlock.svelte'
  import HowItWorksChart from '$lib/components/HowItWorksChart.svelte'
  import ProjectCard from '$lib/components/projects/ProjectCard.svelte'
  import StartSessionPopup from '$lib/components/projects/StartSessionPopup.svelte'
  import { SITE_URL, SITE_NAME, DEFAULT_DESCRIPTION, DEFAULT_OG_IMAGE, pageTitle } from '$lib/seo'
  import { getContext, onMount } from 'svelte'
  import { type Project, type PublicActiveSession } from '$lib/api'
  import { formatHms } from '$lib/format'
  import type { ArenaFrame } from '$lib/types/arena'
  import { UNBOUNDED_RECONNECT, createFrameConnection } from '$lib/ws/connection.svelte'
  const auth = getContext<{ open: (mode: 'login' | 'register') => void }>('auth')

  let { data }: { data: PageData } = $props()

  type OsTab = 'macOS' | 'Linux' | 'Windows'
  let activeTab = $state<OsTab>('macOS')

  const installCommands: Record<OsTab, string> = {
    macOS: 'curl -fsSL https://ololo.dev/install.sh | bash',
    Linux: 'curl -fsSL https://ololo.dev/install.sh | bash',
    Windows: 'powershell -c "irm ololo.dev/install.ps1 | iex"',
  }

  const osTabs: OsTab[] = ['macOS', 'Linux', 'Windows']

  // Only show projects that have a slug (publicly discoverable). Campaign
  // parts are reached through their campaign's page, never listed beside
  // standalone projects — five chapters of one story would crowd out
  // everything else in their category.
  const sluggedProjects = $derived(
    data.projects.filter((p) => p.slug !== null && !p.parent_project_id),
  )

  // Category showcase: up to 3 cards per category, canonical category order
  // first (same order the catalog sidebar uses), uncategorized last. Each
  // group's "See all" deep-links into /projects with the filter preselected.
  const projectsByCategory = $derived.by<{ category: string | null; projects: Project[] }[]>(() => {
    const groups = new Map<string, Project[]>()
    for (const p of sluggedProjects) {
      const key = p.category ?? ''
      if (!groups.has(key)) groups.set(key, [])
      groups.get(key)!.push(p)
    }
    const ordered: { category: string | null; projects: Project[] }[] = []
    for (const cat of data.categories ?? []) {
      if (groups.has(cat)) {
        ordered.push({ category: cat, projects: groups.get(cat)! })
        groups.delete(cat)
      }
    }
    // Categories the canonical list does not know about, in appearance order.
    for (const [cat, list] of groups) {
      if (cat !== '') ordered.push({ category: cat, projects: list })
    }
    if (groups.has('')) ordered.push({ category: null, projects: groups.get('')! })
    return ordered
  })

  // "Start session" popup shared by the landing's project cards.
  let startSlug = $state<string | null>(null)
  let startOpen = $state(false)
  function openStartPopup(slug: string) {
    startSlug = slug
    startOpen = true
  }

  // ── Live sessions ────────────────────────────────────────────────────────
  // The REST snapshot seeds the list (newest first); `/ws/landing/observe`
  // then keeps it current — new sessions land on top, terminal ones drop,
  // player counts move. With none live, Top players keeps the full width.
  // The snapshot is deliberately read once — after mount the WS feed owns the
  // list, and re-reading props would clobber sessions it added or removed.
  // svelte-ignore state_referenced_locally
  const sessionSeed = data.activeSessions ?? []
  let liveSessions = $state<PublicActiveSession[]>(sessionSeed)

  // Per-session countdown seconds, keyed by session id. Seeded from the REST
  // snapshot, ticked down locally every second, and re-anchored by every
  // server countdown frame — so the clock survives without WS but never
  // drifts far with it.
  const initialCountdowns: Record<string, number> = {}
  for (const s of sessionSeed) {
    if (s.seconds_remaining !== null) initialCountdowns[s.id] = s.seconds_remaining
  }
  let countdowns = $state<Record<string, number>>(initialCountdowns)

  function onLandingFrame(frame: ArenaFrame) {
    switch (frame.type) {
      case 'project_session_update': {
        if (frame.status !== 'lobby' && frame.status !== 'running') {
          // Finished/cancelled/paused — no longer landing material (the REST
          // snapshot excludes those too, so a reload agrees).
          liveSessions = liveSessions.filter((s) => s.id !== frame.session_id)
          const { [frame.session_id]: _gone, ...rest } = countdowns
          countdowns = rest
          return
        }
        const existing = liveSessions.find((s) => s.id === frame.session_id)
        if (existing) {
          liveSessions = liveSessions.map((s) =>
            s.id === frame.session_id
              ? {
                  ...s,
                  name: frame.name,
                  status: frame.status as 'lobby' | 'running',
                  players: frame.player_count ?? s.players,
                }
              : s,
          )
          return
        }
        if (!frame.join_code) return
        // A session announced only over WS: project facts come from the
        // public projects the landing already loaded.
        const project = data.projects.find((p) => p.id === frame.project_id)
        liveSessions = [
          {
            id: frame.session_id,
            join_code: frame.join_code,
            name: frame.name,
            status: frame.status as 'lobby' | 'running',
            project_name: project?.name ?? '',
            project_slug: project?.slug ?? null,
            cover_image_url: project?.cover_image_url ?? null,
            players: frame.player_count ?? 0,
            created_at: frame.created_at,
            started_at: null,
            seconds_remaining: null,
          },
          ...liveSessions,
        ]
        break
      }
      case 'lobby_countdown':
      case 'running_countdown':
        countdowns = { ...countdowns, [frame.session_id]: frame.seconds_remaining }
        break
    }
  }

  onMount(() => {
    const conn = createFrameConnection<ArenaFrame>({
      path: '/ws/landing/observe',
      onFrame: onLandingFrame,
      reconnect: UNBOUNDED_RECONNECT,
    })
    conn.connect()
    const tick = setInterval(() => {
      const next: Record<string, number> = {}
      for (const [id, secs] of Object.entries(countdowns)) next[id] = Math.max(0, secs - 1)
      countdowns = next
    }, 1000)
    return () => {
      conn.disconnect()
      clearInterval(tick)
    }
  })

  const jsonLd = JSON.stringify({
    '@context': 'https://schema.org',
    '@type': 'SoftwareApplication',
    name: SITE_NAME,
    url: SITE_URL,
    applicationCategory: 'DeveloperApplication',
    operatingSystem: 'macOS, Linux, Windows',
    description: DEFAULT_DESCRIPTION,
    offers: { '@type': 'Offer', price: '0', priceCurrency: 'USD' },
  })
  // The closing tag is split so the .svelte parser doesn't end the component
  // <script> block early.
  const jsonLdTag = `<script type="application/ld+json">${jsonLd}<` + `/script>`
</script>

<svelte:head>
  <title>{pageTitle()}</title>
  <meta name="description" content={DEFAULT_DESCRIPTION} />
  <link rel="canonical" href={SITE_URL} />
  <meta property="og:type" content="website" />
  <meta property="og:site_name" content={SITE_NAME} />
  <meta property="og:title" content={pageTitle()} />
  <meta property="og:description" content={DEFAULT_DESCRIPTION} />
  <meta property="og:url" content={SITE_URL} />
  <meta property="og:image" content={DEFAULT_OG_IMAGE} />
  <meta name="twitter:card" content="summary_large_image" />
  <meta name="twitter:title" content={pageTitle()} />
  <meta name="twitter:description" content={DEFAULT_DESCRIPTION} />
  <meta name="twitter:image" content={DEFAULT_OG_IMAGE} />
  {@html jsonLdTag}
</svelte:head>

<!-- ===================== HOW IT WORKS ===================== -->
<!-- The demo's opener: one animated session explains the whole game before
     any list of sessions or projects asks the visitor to already know it. -->
<div id="how-it-works" class="-mx-6 -mt-8 scroll-mt-6 bg-brand-light-blue">
  <div class="mx-auto max-w-[1206px] px-[18px] py-10 md:py-[48px]">
    <HowItWorksChart />
  </div>
</div>

<!-- ===================== LIVE SESSIONS ===================== -->
{#if liveSessions.length > 0}
  <div class="-mx-6 bg-brand-light-blue">
    <div class="mx-auto max-w-[1206px] px-[18px] pb-12 md:pb-[88px]">
      <div class="grid grid-cols-1 items-start gap-10">
        {#if liveSessions.length > 0}
          <div data-testid="live-sessions">
            <div class="mb-6 flex items-center justify-between gap-4 md:mb-10">
              <h2 class="flex items-center gap-3 font-heading text-[26px] font-bold leading-8 text-[#363636] md:text-[40px] md:leading-[1.2]">
                <span class="relative flex h-3 w-3" aria-hidden="true">
                  <span class="absolute inline-flex h-full w-full animate-ping rounded-full bg-[#e5484d] opacity-75 motion-reduce:hidden"></span>
                  <span class="relative inline-flex h-3 w-3 rounded-full bg-[#e5484d]"></span>
                </span>
                Live sessions
              </h2>
            </div>

            <div class="overflow-hidden rounded-lg bg-white shadow-sm">
              {#each liveSessions as s (s.id)}
                {@const lobby = s.status === 'lobby'}
                {@const accent = lobby ? '#8fb4ec' : '#fb341c'}
                {@const secs = countdowns[s.id]}
                <a
                  href="/s/{s.join_code}"
                  class="group flex items-center gap-3 border-b border-brand-border/60 py-4 pl-3 pr-4 transition-colors last:border-0 hover:bg-brand-light-blue/30 sm:gap-4 sm:pl-4 sm:pr-6"
                  style="border-left: 8px solid {accent};"
                  data-testid="live-session"
                >
                  <span class="flex min-h-8 w-[92px] shrink-0 items-center">
                    <span
                      class="inline-flex items-center gap-[6px] rounded-full px-[10px] py-[3px] text-[11px] font-bold uppercase tracking-wide"
                      style="background: {lobby ? '#e2ecfc' : '#ffe8e5'}; color: {lobby
                        ? '#4a7fd4'
                        : '#c92912'};"
                    >
                      <span
                        class="inline-block h-[6px] w-[6px] rounded-full {lobby
                          ? ''
                          : 'animate-pulse motion-reduce:animate-none'}"
                        style="background: {accent};"
                      ></span>
                      {lobby ? 'Lobby' : 'Running'}
                    </span>
                  </span>
                  <span class="min-w-0 flex-1 truncate">
                    <span class="text-base font-medium text-[#363636] group-hover:text-[#0269fb]">
                      {s.name}
                    </span>
                    <!-- Default session names mirror the project name — only a
                         differing project name adds information. -->
                    {#if s.project_name && s.project_name.trim() !== s.name.trim()}
                      <span class="text-sm text-[#5c6b7d]"> · {s.project_name}</span>
                    {/if}
                  </span>
                  <span
                    class="shrink-0 rounded bg-[#f2f5fa] px-2 py-[2px] font-mono text-xs font-semibold tracking-wider text-[#5c6b7d]"
                    title="Join code"
                  >
                    {s.join_code}
                  </span>
                  {#if secs !== undefined}
                    <span class="flex shrink-0 items-baseline gap-1.5">
                      <span class="hidden text-xs text-[#9ea7b6] sm:inline">
                        {lobby ? 'starts in' : 'ends in'}
                      </span>
                      <span
                        class="font-mono text-sm font-semibold tabular-nums"
                        style="color: {lobby ? '#4a7fd4' : '#c92912'};"
                      >
                        {formatHms(secs)}
                      </span>
                    </span>
                  {/if}
                  <span class="hidden shrink-0 text-sm tabular-nums text-[#5c6b7d] sm:block">
                    {s.players}
                    {s.players === 1 ? 'player' : 'players'}
                  </span>
                </a>
              {/each}
            </div>
          </div>
        {/if}
      </div>
    </div>
  </div>
{/if}

<!-- ===================== INSTALL ===================== -->
<div id="install" class="-mx-6">
  <div class="mx-auto max-w-[1206px] px-[18px] py-12 md:py-[88px]">
    <div class="mb-8 text-center md:mb-16">
      <h2 class="font-heading text-[26px] font-bold leading-8 text-[#363636] md:text-[40px] md:leading-[1.2]">
        Install the ololo
      </h2>
    </div>

    <!-- OS tab pills -->
    <div class="flex justify-center">
      <ul
        class="flex w-full items-center rounded-[24px] border-2 border-[#dce9fc] sm:w-auto sm:flex-wrap"
      >
        {#each osTabs as tab (tab)}
          <li class="min-w-0 flex-1 sm:flex-none">
            <button
              type="button"
              onclick={() => (activeTab = tab)}
              class="relative w-full cursor-pointer px-2 py-[14px] text-sm font-semibold transition-colors sm:min-w-[142px] sm:px-4 sm:text-base
                {tab === osTabs[0] ? 'rounded-l-[32px]' : ''}
                {tab === osTabs[osTabs.length - 1] ? 'rounded-r-[32px]' : ''}
                {activeTab === tab
                ? 'z-10 rounded-[32px] border-2 border-[#0269fb] bg-white text-[#363636] -m-[2px]'
                : 'text-[#5c6b7d] hover:text-[#363636]'}"
            >
              {tab}
            </button>
          </li>
        {/each}
      </ul>
    </div>

    <!-- Code block -->
    <div class="mt-8 mx-auto max-w-[762px] md:mt-12">
      <CodeBlock code={installCommands[activeTab]} />
    </div>
  </div>
</div>

<!-- ===================== PROJECTS ===================== -->
<div class="-mx-6 bg-brand-light-blue">
  <div class="mx-auto max-w-[1206px] px-[18px] py-12 md:py-[88px]">
    <div class="mb-8 flex items-center justify-between gap-4 md:mb-16">
      <h2 class="font-heading text-[26px] font-bold leading-8 text-[#363636] md:text-[40px] md:leading-[1.2]">
        Projects
      </h2>
      {#if data.isAdmin || data.allowProjectCreation}
        <a
          href="/projects/new"
          class="shrink-0 text-sm font-semibold text-[#0257d8] hover:underline"
        >
          + New project
        </a>
      {/if}
    </div>

    {#if sluggedProjects.length > 0}
      <!-- Same cards as /projects (shared ProjectCard), grouped by category:
           three per group, "See all" jumps into the catalog with that
           category preselected. -->
      {#each projectsByCategory as group, gi (group.category ?? '__other__')}
        <section class={gi > 0 ? 'mt-[48px]' : ''}>
          <div class="mb-[20px] flex items-baseline justify-between gap-4">
            <h3 class="font-heading text-[20px] font-bold text-[#363636]">
              {group.category ?? 'More projects'}
              <span class="ml-1 text-[14px] font-semibold text-[#9ea7b6]">({group.projects.length})</span>
            </h3>
            <a
              href={group.category ? `/projects?category=${encodeURIComponent(group.category)}` : '/projects'}
              class="flex shrink-0 items-center gap-1 text-sm font-semibold text-[#0257d8] hover:underline"
            >
              See all
              <svg class="fill-[#0257d8]" xmlns="http://www.w3.org/2000/svg" width="9" height="10" viewBox="0 0 306 306"><path d="M94.35 0l-35.7 35.7L175.95 153 58.65 270.3l35.7 35.7 153-153z" /></svg>
            </a>
          </div>
          <div class="grid grid-cols-1 gap-[30px] md:grid-cols-2 lg:grid-cols-3">
            {#each group.projects.slice(0, 3) as project, i (project.id)}
              <ProjectCard {project} eager={gi === 0 && i === 0} onStart={openStartPopup} />
            {/each}
          </div>
        </section>
      {/each}
    {:else if data.isAuthenticated}
      <p class="text-sm text-[#9ea7b6]">
        No projects yet.
        {#if data.isAdmin || data.allowProjectCreation}
          <a href="/projects/new" class="text-[#0257d8] hover:underline"
            >Create your first project</a
          >.
        {/if}
      </p>
    {:else}
      <!-- ponytail: zero public projects with slugs. Once any public project
           exists this branch is dead. Show a sign-up CTA instead of fake
           showcase cards. -->
      <div class="flex flex-col items-center gap-4 py-16 text-center">
        <p class="text-base text-[#9ea7b6]">No public projects available yet.</p>
        <button
          type="button"
          onclick={() => auth.open('register')}
          class="text-base font-semibold text-[#0257d8] hover:underline"
        >
          Sign up to create the first one
        </button>
      </div>
    {/if}
  </div>
</div>

<!-- Start session popup for the project cards above -->
<StartSessionPopup slug={startSlug ?? ""} bind:open={startOpen} />
