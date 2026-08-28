<script lang="ts">
  import '../app.css';
  import AppHeader from '$lib/components/AppHeader.svelte';
  import AppFooter from '$lib/components/AppFooter.svelte';
  import Notifications from '$lib/components/Notifications.svelte';
  import AuthModal from '$lib/components/AuthModal.svelte';
  import { invalidateAll, goto } from '$app/navigation';
  import { browser } from '$app/environment';
  import { initAuth, clearToken } from '$lib/auth';
  import { notify } from '$lib/notifications.svelte';
  import { setContext } from 'svelte';
  import { page } from '$app/stores';

  let { children, data } = $props();

  const user = $derived(data.user ?? { initials: '?', name: '' });

  let authOpen = $state(false);
  let authMode = $state<'login' | 'register'>('login');
  let authError = $state('');

  function openAuth(mode: 'login' | 'register') {
    authMode = mode;
    authOpen = true;
  }

  setContext('auth', { open: openAuth });

  // Initialise the localStorage token on every navigation.
  // `initAuth` short-circuits when a valid token is already in memory.
  $effect(() => {
    if (browser && data.isAuthenticated) {
      initAuth();
    }
  });

  $effect(() => {
    if (browser) {
      const err = $page.url.searchParams.get('auth_error');
      if (err) {
        authError = err;
        openAuth('login');
        // Remove the param so a subsequent page-store update doesn't re-open the modal.
        const url = new URL(window.location.href);
        url.searchParams.delete('auth_error');
        history.replaceState({}, '', url);
      }
    }
  });

  // The email-verification link redirects here with ?emailVerified=1|0.
  $effect(() => {
    if (browser) {
      const verified = $page.url.searchParams.get('emailVerified');
      if (verified !== null) {
        if (verified === '1') {
          notify.success('Email confirmed — your account can now receive sign-in links.');
        } else {
          notify.error('That confirmation link is invalid or has expired. Request a new one from your profile.');
        }
        const url = new URL(window.location.href);
        url.searchParams.delete('emailVerified');
        history.replaceState({}, '', url);
      }
    }
  });
</script>

<AppHeader
  isAuthenticated={data.isAuthenticated}
  isAdmin={data.isAdmin}
  allowProjectCreation={data.allowProjectCreation ?? false}
  {user}
  onLogin={() => openAuth('login')}
  onSignup={() => openAuth('register')}
  onLogout={async () => {
    clearToken();
    await fetch('/logout', {
      method: 'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body: '',
    });
    await invalidateAll();
    goto('/');
  }}
/>

<main class="min-h-screen px-6 py-8">
  {@render children()}
</main>

<AppFooter />

<Notifications />

{#if browser}
  <AuthModal
    open={authOpen}
    mode={authMode}
    initialError={authError}
    onclose={() => { authOpen = false; authError = ''; }}
  />
{/if}
