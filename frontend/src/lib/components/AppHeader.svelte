<script lang="ts">
  import { page } from '$app/stores';
  import { docNav } from '$lib/doc-nav.svelte';
  import AppHeaderDesktopAuth from '$lib/components/AppHeaderDesktopAuth.svelte';
  import AppHeaderMobileMenu from '$lib/components/AppHeaderMobileMenu.svelte';

  interface NavItem {
    label: string;
    href: string;
    active?: boolean;
  }

  interface UserInfo {
    initials: string;
    name: string;
    avatarUrl?: string;
    username?: string;
  }

  interface Props {
    navItems?: NavItem[];
    isAuthenticated?: boolean;
    isAdmin?: boolean;
    allowProjectCreation?: boolean;
    user?: UserInfo;
    onLogin?: () => void;
    onSignup?: () => void;
    onLogout?: () => void;
  }

  let {
    navItems = undefined,
    isAuthenticated = false,
    isAdmin = false,
    allowProjectCreation = false,
    user = { initials: 'AK', name: 'Andrey Kucherenko' },
    onLogin,
    onSignup,
    onLogout,
  }: Props = $props();

  // Explicitly-passed items win (Storybook); otherwise the default nav.
  const effectiveNavItems = $derived(
    navItems ?? [
      { label: 'Projects', href: '/projects' },
      { label: 'Install', href: '/#install' },
      { label: 'Documentation', href: '/documentation' },
    ],
  );

  const showNewProject = $derived(isAdmin || (isAuthenticated && allowProjectCreation));

  let menuOpen = $state(false);

  function toggleMenu() {
    menuOpen = !menuOpen;
  }

  const isDocRoute = $derived($page.url.pathname.startsWith('/documentation'));
  const activeSlug = $derived($page.params.slug ?? '');
  const docSections = $derived(docNav.sections);

  /**
   * Does `href` name the section the current route belongs to?
   *
   * Sub-routes count, so a project detail page keeps "Projects" lit. Hash
   * links ("/#install") never do: they point at a section of the home page
   * rather than a destination of their own, and matching them on pathname
   * would light "Install" on every home-page visit.
   */
  function isCurrentSection(href: string, pathname: string): boolean {
    if (!href.startsWith('/') || href.includes('#')) return false;
    if (href === '/') return pathname === '/';
    return pathname === href || pathname.startsWith(`${href}/`);
  }

  // Items with `active` resolved from the route. An explicitly-passed item
  // wins, so callers (and Storybook) can still force a state.
  const resolvedNavItems = $derived(
    effectiveNavItems.map((item) => ({
      ...item,
      active: item.active ?? isCurrentSection(item.href, $page.url.pathname),
    })),
  );
</script>

<header class="relative z-50 py-6">
  <div class="mx-auto flex max-w-7xl items-center justify-between px-6">

    <!-- Logo -->
    <a href="/" class="relative z-20 shrink-0">
      <img src="/logo.svg" alt="ololo.dev" class="h-10 w-10 md:h-14 md:w-14" />
    </a>

    <!-- Mobile hamburger (shown only below md) -->
    <button
      id="mobile-menu-toggle"
      class="relative z-20 flex h-8 w-8 flex-col items-center justify-center gap-[6px] md:hidden"
      class:invisible={menuOpen}
      type="button"
      aria-label="Toggle menu"
      aria-expanded={menuOpen}
      onclick={toggleMenu}
    >
      <span
        class="block h-0.5 w-6 bg-brand-text transition-transform duration-300"
        class:rotate-45={menuOpen}
        class:translate-y-[7px]={menuOpen}
      ></span>
      <span
        class="block h-0.5 w-6 bg-brand-text transition-opacity duration-300"
        class:opacity-0={menuOpen}
      ></span>
      <span
        class="block h-0.5 w-6 bg-brand-text transition-transform duration-300"
        class:-rotate-45={menuOpen}
        class:-translate-y-[7px]={menuOpen}
      ></span>
    </button>

    <!-- ── Desktop nav (md+) ─────────────────────────────────────────────── -->
    <nav class="hidden md:block mx-auto px-2.5 font-heading font-semibold">
      <ul class="flex flex-wrap items-center gap-8">
        {#each resolvedNavItems as item (item.href)}
          <li>
            <!-- The current section is marked by an underline as well as the
                 accent colour: colour alone is not a distinction everyone can
                 see (WCAG 1.4.1). `aria-current` carries it to screen readers. -->
            <a
              href={item.href}
              aria-current={item.active ? 'page' : undefined}
              class="text-brand-text transition-colors hover:text-brand-blue"
              class:text-brand-blue={item.active}
              class:underline={item.active}
              class:underline-offset-8={item.active}
              class:decoration-2={item.active}
            >
              {item.label}
            </a>
          </li>
        {/each}
      </ul>
    </nav>

    <!-- ── Desktop auth (md+) ────────────────────────────────────────────── -->
    <AppHeaderDesktopAuth
      {isAuthenticated}
      {isAdmin}
      {showNewProject}
      {user}
      {onLogin}
      {onSignup}
      {onLogout}
    />

    <!-- ── Mobile full-screen overlay (<md) ─────────────────────────────── -->
    <AppHeaderMobileMenu
      bind:menuOpen
      navItems={resolvedNavItems}
      {isAuthenticated}
      {isAdmin}
      {showNewProject}
      {user}
      {onLogin}
      {onSignup}
      {onLogout}
      {isDocRoute}
      {docSections}
      {activeSlug}
    />

  </div>
</header>