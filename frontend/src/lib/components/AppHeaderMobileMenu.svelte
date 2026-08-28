<script lang="ts">
  interface NavItem {
    label: string;
    href: string;
    active?: boolean;
  }

  interface UserInfo {
    name: string;
    username?: string;
  }

  interface DocSection {
    id: string;
    label: string;
    items: { slug: string; title: string }[];
  }

  let {
    menuOpen = $bindable(),
    navItems,
    isAuthenticated,
    isAdmin,
    showNewProject,
    user,
    onLogin,
    onSignup,
    onLogout,
    isDocRoute,
    docSections,
    activeSlug,
  }: {
    menuOpen: boolean;
    navItems: NavItem[];
    isAuthenticated: boolean;
    isAdmin: boolean;
    showNewProject: boolean;
    user: UserInfo;
    onLogin?: () => void;
    onSignup?: () => void;
    onLogout?: () => void;
    isDocRoute: boolean;
    docSections: DocSection[];
    activeSlug: string;
  } = $props();

  let overlayEl: HTMLElement | null = $state(null);
  let mobileDocSectionsOpen = $state(new Set<string>());

  const activeDocSection = $derived.by(() => {
    for (const s of docSections) {
      for (const item of s.items) {
        if (item.slug === activeSlug) return s.id;
      }
    }
    return '';
  });

  function isDocSectionOpen(sectionId: string): boolean {
    return mobileDocSectionsOpen.has(sectionId) || activeDocSection === sectionId;
  }

  function toggleDocSection(sectionId: string) {
    if (activeDocSection === sectionId) return;
    const next = new Set(mobileDocSectionsOpen);
    if (next.has(sectionId)) {
      next.delete(sectionId);
    } else {
      next.add(sectionId);
    }
    mobileDocSectionsOpen = next;
  }

  function closeMobileMenu() {
    menuOpen = false;
    mobileDocSectionsOpen = new Set();
  }

  function handleOverlayKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      closeMobileMenu();
      document.getElementById('mobile-menu-toggle')?.focus();
    }
  }

  function handleOverlayFocusTrap(e: FocusEvent) {
    // The handler stays bound while the overlay is closed (opacity-0 in the
    // DOM) and during the tap that closes it — trapping then would steal
    // focus from whatever the user moved on to (e.g. the auth modal's
    // inputs, which on mobile made the form impossible to type into).
    if (!menuOpen || !overlayEl) return;
    const focusable = overlayEl.querySelectorAll<HTMLElement>(
      'a[href], button:not([disabled]), [tabindex]:not([tabindex="-1"])'
    );
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (e.target === last && e.relatedTarget === first) return;
    if (e.target === first && e.relatedTarget === last) return;
    if (!overlayEl.contains(e.relatedTarget as Node)) {
      first.focus();
    }
  }

  $effect(() => {
    if (typeof document !== 'undefined') {
      document.body.classList.toggle('overflow-hidden', menuOpen);
    }
    if (menuOpen) {
      requestAnimationFrame(() => overlayEl?.focus());
    } else {
      mobileDocSectionsOpen = new Set();
    }
  });
</script>

<div
  bind:this={overlayEl}
  role="dialog"
  aria-modal="true"
  aria-label="Navigation menu"
  tabindex="-1"
  class="pointer-events-none fixed inset-0 z-10 flex flex-col bg-white opacity-0 transition-opacity duration-300 md:hidden"
  class:pointer-events-auto={menuOpen}
  class:opacity-100={menuOpen}
  onkeydown={handleOverlayKeydown}
  onfocusout={handleOverlayFocusTrap}
>
  <!-- Overlay header: user info + close button -->
  <div class="flex shrink-0 items-center justify-between border-b border-brand-border px-6 py-4">
    {#if isAuthenticated}
      <a
        href={user.username ? `/u/${user.username}` : '/profile'}
        onclick={closeMobileMenu}
        class="flex min-w-0 flex-col"
      >
        <span class="font-heading text-sm font-semibold text-brand-text leading-snug truncate">{user.name}</span>
        {#if user.username}
          <span class="font-mono text-xs text-brand-muted leading-snug truncate">@{user.username}</span>
        {/if}
      </a>
    {:else}
      <a href="/" onclick={closeMobileMenu}>
        <img src="/logo.svg" alt="ololo.dev" class="h-10 w-10" />
      </a>
    {/if}
    <button
      type="button"
      onclick={closeMobileMenu}
      aria-label="Close menu"
      class="ml-4 flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-brand-muted transition-colors hover:bg-brand-light-blue hover:text-brand-text"
    >
      <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <line x1="18" y1="6" x2="6" y2="18"></line>
        <line x1="6" y1="6" x2="18" y2="18"></line>
      </svg>
    </button>
  </div>

  <!-- Overlay scrollable body -->
  <div class="flex flex-1 flex-col overflow-y-auto pb-8">

    <!-- Nav links -->
    <nav class="px-6 pt-2 pb-1 font-heading font-semibold">
      <ul>
        {#each navItems as item (item.href)}
          <li class="border-b border-brand-border last:border-b-0">
            <a
              href={item.href}
              onclick={closeMobileMenu}
              aria-current={item.active ? 'page' : undefined}
              class="flex items-center py-4 text-base text-brand-text transition-colors hover:text-brand-blue"
              class:text-brand-blue={item.active}
              class:font-bold={item.active}
            >
              {item.label}
            </a>
          </li>
        {/each}
      </ul>
    </nav>

    <!-- Account / auth section -->
    {#if isAuthenticated}
      <div class="mx-6 mt-4 border-t border-brand-border"></div>
      <div class="px-6 pt-2 pb-1 font-heading font-semibold">
        <p class="py-3 text-xs font-semibold uppercase tracking-wider text-brand-muted">Account</p>
        <ul>
          {#if showNewProject}
            <li class="border-b border-brand-border">
              <a href="/projects/new" onclick={closeMobileMenu} class="flex items-center py-4 text-base text-brand-blue transition-colors hover:text-brand-text">
                New Project
              </a>
            </li>
          {/if}
          {#if isAdmin}
            <li class="border-b border-brand-border">
              <a href="/settings" onclick={closeMobileMenu} class="flex items-center py-4 text-base text-brand-blue transition-colors hover:text-brand-text">
                Settings
              </a>
            </li>
          {/if}
          <li class="border-b border-brand-border">
            <a href="/projects" onclick={closeMobileMenu} class="flex items-center py-4 text-base text-brand-text transition-colors hover:text-brand-blue">
              My Projects
            </a>
          </li>
          {#if user.username}
            <li class="border-b border-brand-border">
              <a href="/u/{user.username}" onclick={closeMobileMenu} class="flex items-center py-4 text-base text-brand-text transition-colors hover:text-brand-blue">
                My Sessions
              </a>
            </li>
          {/if}
          <li class="border-b border-brand-border">
            <a href="/profile" onclick={closeMobileMenu} class="flex items-center py-4 text-base text-brand-text transition-colors hover:text-brand-blue">
              Profile
            </a>
          </li>
          <li>
            <button
              type="button"
              class="flex items-center py-4 text-base text-brand-muted transition-colors hover:text-brand-text bg-transparent font-heading font-semibold"
              onclick={() => { closeMobileMenu(); onLogout?.(); }}
            >
              Log Out
            </button>
          </li>
        </ul>
      </div>
    {:else}
      <div class="px-6 pt-6 pb-2 flex flex-col gap-3">
        <!-- Close the overlay BEFORE opening the auth modal: an open menu
             keeps its focus trap alive, which yanks focus out of the modal's
             inputs on every tap — on mobile that reads as "the form does not
             accept input". -->
        <button
          type="button"
          class="flex w-full items-center justify-center rounded-btn border-2 border-brand-blue bg-transparent px-5 py-3 font-heading text-base font-bold text-brand-blue transition-colors hover:bg-brand-blue hover:text-white"
          onclick={() => { closeMobileMenu(); onLogin?.(); }}
        >
          Log In
        </button>
        <button
          type="button"
          class="flex w-full items-center justify-center rounded-btn border-2 border-brand-blue bg-brand-blue px-5 py-3 font-heading text-base font-bold text-white transition-colors hover:bg-transparent hover:text-brand-blue"
          onclick={() => { closeMobileMenu(); onSignup?.(); }}
        >
          Sign Up
        </button>
      </div>
    {/if}

    <!-- Doc section navigation (on doc routes) -->
    {#if isDocRoute && docSections.length > 0}
      <div class="mx-6 mt-4 border-t border-brand-border"></div>
      <div class="px-6 pt-2 pb-1">
        <p class="py-3 text-xs font-semibold uppercase tracking-wider text-brand-muted font-heading">Documentation</p>
        <ul>
          {#each docSections as section (section.id)}
            {@const sectionOpen = isDocSectionOpen(section.id)}
            {@const sectionActive = activeDocSection === section.id}
            <li class="border-b border-brand-border last:border-b-0">
              <button
                type="button"
                onclick={() => toggleDocSection(section.id)}
                class="flex w-full items-center justify-between py-4 text-left text-base font-semibold font-heading transition-colors
                  {sectionActive ? 'text-brand-text' : 'text-brand-check-blue hover:text-brand-text'}"
              >
                <span>{section.label}</span>
                <svg
                  xmlns="http://www.w3.org/2000/svg"
                  fill="#9ea7b6"
                  width="12"
                  height="12"
                  viewBox="0 0 306 306"
                  style="transform: rotate({sectionOpen ? '-90deg' : '90deg'}); transition: transform .3s; flex-shrink: 0;"
                >
                  <path d="M94.35 0l-35.7 35.7L175.95 153 58.65 270.3l35.7 35.7 153-153z" />
                </svg>
              </button>
              {#if sectionOpen}
                <ul class="pb-2">
                  {#each section.items as item (item.slug)}
                    <li>
                      <a
                        href="/documentation/{item.slug}"
                        onclick={closeMobileMenu}
                        class="flex items-center py-3 pl-4 text-sm transition-colors
                          {activeSlug === item.slug
                          ? 'font-semibold text-brand-blue'
                          : 'text-brand-text hover:text-brand-blue'}"
                      >
                        {item.title}
                      </a>
                    </li>
                  {/each}
                </ul>
              {/if}
            </li>
          {/each}
        </ul>
      </div>
    {/if}

  </div>
</div>