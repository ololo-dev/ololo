<script lang="ts">
  import { ikAvatar } from '$lib/imagekit';
  interface UserInfo {
    initials: string;
    name: string;
    avatarUrl?: string;
    username?: string;
  }

  let {
    isAuthenticated,
    isAdmin,
    showNewProject,
    user,
    onLogin,
    onSignup,
    onLogout,
  }: {
    isAuthenticated: boolean;
    isAdmin: boolean;
    showNewProject: boolean;
    user: UserInfo;
    onLogin?: () => void;
    onSignup?: () => void;
    onLogout?: () => void;
  } = $props();

  let adminMenuOpen = $state(false);
  let dropdownEl: HTMLElement | null = $state(null);
  let toggleBtnEl: HTMLElement | null = $state(null);

  function toggleAdminMenu() {
    adminMenuOpen = !adminMenuOpen;
  }

  function closeAdminMenu() {
    adminMenuOpen = false;
  }

  $effect(() => {
    if (!adminMenuOpen) return;

    function handleClickOutside(e: MouseEvent) {
      const target = e.target as Node;
      const clickedInsideDropdown = dropdownEl?.contains(target) ?? false;
      const clickedToggleBtn = toggleBtnEl?.contains(target) ?? false;
      if (!clickedInsideDropdown && !clickedToggleBtn) {
        adminMenuOpen = false;
      }
    }

    document.addEventListener('click', handleClickOutside);
    return () => document.removeEventListener('click', handleClickOutside);
  });
</script>

{#if isAuthenticated}
  <div class="relative hidden md:block">
    <button
      type="button"
      bind:this={toggleBtnEl}
      class="flex items-center gap-2 bg-transparent"
      onclick={toggleAdminMenu}
    >
      {#if user.avatarUrl}
        <img src={ikAvatar(user.avatarUrl, 56)} alt={user.name} class="h-14 w-14 rounded-full object-cover" />
      {:else}
        <div class="flex h-14 w-14 items-center justify-center rounded-full bg-brand-light-blue font-heading text-base font-semibold uppercase text-brand-check-blue">
          {user.initials}
        </div>
      {/if}
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width="11"
        height="12"
        viewBox="0 0 306 306"
        class="rotate-90 fill-brand-muted transition-transform duration-300"
        class:rotate-[270deg]={adminMenuOpen}
      >
        <path d="M94.35 0l-35.7 35.7L175.95 153 58.65 270.3l35.7 35.7 153-153z" />
      </svg>
    </button>

    {#if adminMenuOpen}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        bind:this={dropdownEl}
        class="absolute right-0 top-full z-50 mt-2 w-64 rounded-btn bg-white shadow-[0_6px_32px_0_rgba(19,101,218,0.16)]"
      >
        <div class="border-b-2 border-brand-light-blue p-7">
          <a
            href={user.username ? `/u/${user.username}` : '/profile'}
            onclick={closeAdminMenu}
            class="group flex w-full min-w-0 flex-col"
          >
            <span class="font-heading text-base font-semibold text-brand-text leading-snug truncate transition-colors group-hover:text-brand-blue">{user.name}</span>
            {#if user.username}
              <span class="font-mono text-sm text-brand-muted leading-snug truncate">@{user.username}</span>
            {/if}
          </a>
        </div>
        <div class="border-b-2 border-brand-light-blue p-7">
          <ul class="mt-2 space-y-4">
            {#if showNewProject}
              <li>
                <a href="/projects/new" onclick={closeAdminMenu} class="font-heading font-semibold text-brand-blue transition-colors hover:text-brand-text">
                  New Project
                </a>
              </li>
            {/if}
            {#if isAdmin}
              <li>
                <a href="/settings" onclick={closeAdminMenu} class="font-heading font-semibold text-brand-blue transition-colors hover:text-brand-text">
                  Settings
                </a>
              </li>
            {/if}
            <li>
              <a href="/projects" onclick={closeAdminMenu} class="font-heading font-semibold text-brand-text transition-colors hover:text-brand-blue">
                My Projects
              </a>
            </li>
            {#if user.username}
              <li>
                <a href="/u/{user.username}" onclick={closeAdminMenu} class="font-heading font-semibold text-brand-text transition-colors hover:text-brand-blue">
                  My Sessions
                </a>
              </li>
            {/if}
            <li>
              <a href="/profile" onclick={closeAdminMenu} class="font-heading font-semibold text-brand-text transition-colors hover:text-brand-blue">
                Profile
              </a>
            </li>
          </ul>
        </div>
        <div class="p-7">
          <button
            class="font-heading font-semibold text-brand-text hover:text-brand-blue transition-colors bg-transparent"
            type="button"
            onclick={() => { closeAdminMenu(); onLogout?.(); }}
          >
            Log Out
          </button>
        </div>
      </div>
    {/if}
  </div>
{:else}
  <div class="hidden md:flex items-center gap-7">
    <button
      type="button"
      class="inline-flex min-w-[120px] items-center justify-center rounded-btn border-2 border-brand-blue bg-transparent px-5 py-[10px] font-heading text-base font-bold text-brand-blue transition-colors hover:bg-brand-blue hover:text-white"
      onclick={onLogin}
    >
      Log In
    </button>
    <button
      type="button"
      class="inline-flex min-w-[120px] items-center justify-center rounded-btn border-2 border-brand-blue bg-brand-blue px-5 py-[10px] font-heading text-base font-bold text-white transition-colors hover:bg-transparent hover:text-brand-blue"
      onclick={onSignup}
    >
      Sign Up
    </button>
  </div>
{/if}