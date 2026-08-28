<script lang="ts">
  import { page } from '$app/stores';
  import { browser } from '$app/environment';
  import type { Snippet } from 'svelte';
  import SettingsSidebar from '$lib/components/settings/SettingsSidebar.svelte';

  let { data, children }: {
    data: {
      userCount: number;
      projectCount: number;
      categoryCount: number;
      judgeCount: number;
      sessionCount: number;
    };
    children: Snippet;
  } = $props();

  type Tab = { id: string; label: string; href: string };
  const tabs: Tab[] = [
    { id: 'overview', label: 'Overview', href: '/settings' },
    { id: 'general', label: 'General', href: '/settings/general' },
    { id: 'ai', label: 'AI', href: '/settings/ai' },
    { id: 'telemetry', label: 'Telemetry', href: '/settings/telemetry' },
    { id: 'analytics', label: 'Costs', href: '/settings/analytics' },
    { id: 'projects', label: 'Projects', href: '/settings/projects' },
    { id: 'sessions', label: 'Sessions', href: '/settings/sessions' },
    { id: 'users', label: 'Users', href: '/settings/users' },
    { id: 'game_servers', label: 'Game Servers', href: '/settings/game-servers' },
    { id: 'email', label: 'Email', href: '/settings/email' },
    { id: 'categories', label: 'Categories', href: '/settings/categories' },
    { id: 'judges', label: 'Judges', href: '/settings/judges' },
  ];

  // Bare /settings is the Overview dashboard.
  // Normalize hyphens to underscores so 'game-servers' matches tab id 'game_servers'.
  const activeId = $derived(
    $page.url.pathname === '/settings'
      ? 'overview'
      : $page.url.pathname.replace('/settings/', '').replace(/-/g, '_')
  );

  function badgeCount(tabId: string): number | undefined {
    if (tabId === 'users') return data.userCount;
    if (tabId === 'projects') return data.projectCount;
    if (tabId === 'sessions') return data.sessionCount;
    if (tabId === 'categories') return data.categoryCount;
    if (tabId === 'judges') return data.judgeCount;
    return undefined;
  }

  const sidebarTabs = $derived(
    tabs.map((tab) => ({ ...tab, count: badgeCount(tab.id) })),
  );

  // Remembered per browser: an admin who works in one tab should not have to
  // re-collapse the rail on every visit. Read lazily — this runs during SSR
  // too, where there is no localStorage.
  const STORAGE_KEY = 'settings:sidebar-collapsed';
  let collapsed = $state(browser && localStorage.getItem(STORAGE_KEY) === '1');

  function setCollapsed(value: boolean) {
    collapsed = value;
    if (browser) localStorage.setItem(STORAGE_KEY, value ? '1' : '0');
  }
</script>

<svelte:head>
  <title>Settings — ololo.dev</title>
</svelte:head>

<div class="-mx-6 -mt-8 min-h-screen bg-brand-light-blue">
  <div class="mx-auto w-full max-w-[1206px] px-[18px] py-10 md:py-[88px]">

    <h1 class="font-heading text-[34px] font-bold leading-[1.18] text-brand-text">
      Admin Settings
    </h1>

    <!-- Two columns from md up: navigation on the left, the section itself
         on the right. Stacked on a phone, where a rail would eat the page. -->
    <div class="mt-8 flex flex-col gap-[18px] md:flex-row md:items-start">
      <SettingsSidebar
        tabs={sidebarTabs}
        {activeId}
        {collapsed}
        onToggle={setCollapsed}
      />

      <div class="min-w-0 flex-1">
        {@render children()}
      </div>
    </div>
  </div>
</div>