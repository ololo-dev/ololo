<script lang="ts">
  import { getContext } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/stores';

  let { data } = $props();

  const auth = getContext<{ open: (mode: 'login' | 'register') => void }>('auth');

  // Plain variable — not reactive state, used as a guard so the modal
  // only auto-opens once per page visit.
  let modalOpened = false;

  $effect(() => {
    const next = $page.url.searchParams.get('next') ?? '/';
    if (data.isAuthenticated) {
      // Already authenticated (or just logged in via the modal).
      goto(next.startsWith('/') ? next : '/');
    } else if (!modalOpened) {
      modalOpened = true;
      auth.open('login');
    }
  });
</script>

<svelte:head>
  <title>Log in — ololo.dev</title>
</svelte:head>
