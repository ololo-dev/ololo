<script lang="ts">
  import { onDestroy } from 'svelte';
  import { page } from '$app/stores';

  let status = $state(500);
  let message = $state('An unexpected error occurred.');

  let unsubscribe: (() => void) | undefined;
  try {
    unsubscribe = page.subscribe((p) => {
      status = p.status;
      message = p.error?.message ?? 'An unexpected error occurred.';
    });
  } catch {
    // Store context unavailable in this render path — defaults above are used.
  }

  onDestroy(() => unsubscribe?.());

  const is404 = $derived(status === 404);

  const heading = $derived(is404 ? 'Page not found' : `Error ${status}`);
  const body = $derived(
    is404
      ? 'The page you are looking for might have been removed, had its name changed, or is temporarily unavailable.'
      : message
  );
</script>

<svelte:head>
  <title>{status} — ololo.dev</title>
</svelte:head>

<div class="-mx-6 -mt-8 flex min-h-[calc(100vh-192px)] items-center bg-brand-light-blue px-6 py-[88px]">
  <div class="mx-auto flex w-full max-w-[1206px] gap-8 px-[18px]">

    <!-- left: text -->
    <div class="flex flex-[50%] items-start pt-20">
      <div class="max-w-[370px]">
        <h1 class="mb-8 font-heading text-[42px] font-bold leading-tight text-[#363636]">{heading}</h1>
        <p class="text-base leading-relaxed text-[#363636]">{body}</p>
        <div class="mt-12">
          <a
            href="/"
            class="inline-block rounded bg-[#0269fb] px-6 py-3 text-sm font-semibold text-white transition-opacity hover:opacity-90"
          >
            Go to Homepage
          </a>
        </div>
      </div>
    </div>

    <!-- right: illustration -->
    <div class="flex flex-[50%] items-center justify-center">
      <img src="/maskot-error.png" alt="Error illustration" class="block max-w-full" />
    </div>

  </div>
</div>
