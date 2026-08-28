<!--
  Settings navigation as a left rail.

  Eleven tabs stopped fitting a single row a while ago — the old layout put
  them in a horizontally scrolling strip, which hides the tail of the list
  behind a gesture. A column shows all of them at once, and collapses to
  icons when the content needs the width.

  On a phone the rail becomes a disclosure: the open section is shown, and
  one tap reveals the rest. A scrolling strip would hide its own tail behind
  a sideways gesture — the very thing the rail is here to fix.
-->
<script lang="ts">
  import {
    Activity,
    Bot,
    ChevronDown,
    CircleDollarSign,
    FolderKanban,
    Gavel,
    LayoutDashboard,
    Mail,
    PanelLeftClose,
    PanelLeftOpen,
    Play,
    Server,
    Settings,
    Tags,
    Users,
    type Icon as IconType,
  } from 'lucide-svelte';

  export interface SettingsTab {
    id: string;
    label: string;
    href: string;
    /** Shown as a pill after the label; omitted when there is nothing to count. */
    count?: number;
  }

  interface Props {
    tabs: SettingsTab[];
    /** Tab id currently open. */
    activeId: string;
    collapsed?: boolean;
    onToggle?: (collapsed: boolean) => void;
  }

  let { tabs, activeId, collapsed = false, onToggle }: Props = $props();

  // Phones get a disclosure rather than a scrolling strip: with eleven
  // sections the strip hides its own tail behind a sideways gesture, which
  // is the problem the rail exists to solve.
  let mobileOpen = $state(false);
  const active = $derived(tabs.find((t) => t.id === activeId) ?? tabs[0]);

  const ICONS: Record<string, typeof IconType> = {
    overview: LayoutDashboard,
    general: Settings,
    ai: Bot,
    telemetry: Activity,
    analytics: CircleDollarSign,
    projects: FolderKanban,
    sessions: Play,
    users: Users,
    game_servers: Server,
    email: Mail,
    categories: Tags,
    judges: Gavel,
  };
</script>

<nav
  class="shrink-0 md:sticky md:top-[24px] {collapsed ? 'md:w-[60px]' : 'md:w-[212px]'}"
  aria-label="Settings sections"
  data-testid="settings-sidebar"
  data-collapsed={collapsed}
>
  <div class="rounded-[10px] bg-white p-1 shadow-sm">
    <!-- The toggle is desktop-only: on a phone the rail is already a strip. -->
    <div class="hidden justify-end px-1 pb-1 pt-0.5 md:flex">
      <button
        type="button"
        onclick={() => onToggle?.(!collapsed)}
        class="rounded-[6px] p-1.5 text-brand-muted transition-colors hover:bg-brand-light-blue
               hover:text-brand-text"
        aria-label={collapsed ? 'Expand settings menu' : 'Collapse settings menu'}
        aria-expanded={!collapsed}
        title={collapsed ? 'Expand' : 'Collapse'}
        data-testid="sidebar-toggle"
      >
        {#if collapsed}
          <PanelLeftOpen size={16} />
        {:else}
          <PanelLeftClose size={16} />
        {/if}
      </button>
    </div>

    <!-- Phone: the open section, tap to reveal the rest. -->
    {#if active}
      {@const ActiveIcon = ICONS[active.id] ?? Settings}
      <button
        type="button"
        onclick={() => (mobileOpen = !mobileOpen)}
        aria-expanded={mobileOpen}
        data-testid="sidebar-mobile-toggle"
        class="flex w-full items-center gap-2 rounded-[7px] px-3 py-2 text-sm font-semibold
               text-brand-text md:hidden"
      >
        <ActiveIcon size={16} class="shrink-0" />
        <span>{active.label}</span>
        <ChevronDown
          size={16}
          class="ml-auto shrink-0 text-brand-muted transition-transform duration-150
                 {mobileOpen ? 'rotate-180' : ''}"
        />
      </button>
    {/if}

    <div
      class="flex-col gap-1 md:flex {mobileOpen ? 'flex' : 'hidden'}"
      data-testid="settings-tab-list"
    >
      {#each tabs as tab (tab.id)}
        {@const Icon = ICONS[tab.id] ?? Settings}
        {@const isActive = activeId === tab.id}
        <a
          href={tab.href}
          aria-current={isActive ? 'page' : undefined}
          title={collapsed ? tab.label : undefined}
          data-testid="settings-tab-{tab.id}"
          class="flex shrink-0 items-center gap-2 whitespace-nowrap rounded-[7px] px-3 py-2
                 text-sm font-semibold transition-colors duration-150
                 {collapsed ? 'md:justify-center md:px-2' : ''}
                 {isActive
            ? 'bg-brand-blue text-white shadow-sm'
            : 'text-brand-muted hover:bg-brand-light-blue hover:text-brand-text'}"
        >
          <Icon size={16} class="shrink-0" />
          <span class={collapsed ? 'md:hidden' : ''}>{tab.label}</span>
          {#if tab.count !== undefined}
            <span
              class="ml-auto rounded-full px-1.5 py-px text-[10px] font-bold
                     {collapsed ? 'md:hidden' : ''}
                     {isActive ? 'bg-white/20 text-white' : 'bg-brand-blue/10 text-brand-blue'}"
            >
              {tab.count}
            </span>
          {/if}
        </a>
      {/each}
    </div>
  </div>
</nav>
