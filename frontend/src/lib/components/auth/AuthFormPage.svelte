<script lang="ts">
  import { enhance } from '$app/forms';
  import Turnstile from '$lib/components/Turnstile.svelte';
  import type { ActionResult } from '@sveltejs/kit';
  import type { Snippet } from 'svelte';

  // The one card shared by every standalone auth page (forgot-password,
  // magic-link, reset-password): centered white card on the light-blue
  // ground, a form the shell wires with enhance + captcha + error line +
  // submit button, and a success view that replaces it. The page supplies
  // its copy and its input fields; result interpretation is injectable
  // because the actions answer differently.
  type Props = {
    /** Browser-tab title. */
    title: string;
    heading: string;
    description?: string;
    submitLabel: string;
    /** Button copy while the action is in flight. */
    loadingLabel: string;
    turnstile: { enabled: boolean; sitekey: string | null };
    /** The form's inputs; everything around them is the shell's business. */
    fields: Snippet;
    successHeading: string;
    successText: string;
    /** Keep the back-to-login link on the success view (drop it when the
     *  page redirects away on its own). */
    successBackLink?: boolean;
    /** Message for a failed action result, from the server error code. */
    failureMessage?: (code: string) => string;
    /** Whether a non-failure result counts as success (default: always). */
    resultIsSuccess?: (result: ActionResult) => boolean;
    /** Pre-submit guard: return an error message to abort the submit. */
    guard?: () => string | null;
  };
  let {
    title,
    heading,
    description,
    submitLabel,
    loadingLabel,
    turnstile,
    fields,
    successHeading,
    successText,
    successBackLink = false,
    failureMessage,
    resultIsSuccess,
    guard,
  }: Props = $props();

  let done = $state(false);
  let loading = $state(false);
  let error = $state('');
  let turnstileToken = $state('');
  let turnstileRef = $state<{ reset: () => void } | null>(null);

  const captchaPending = $derived(!!(turnstile.enabled && turnstile.sitekey) && !turnstileToken);

  const handleSubmit = ({ cancel }: { cancel: () => void }) => {
    const guardError = guard?.() ?? null;
    if (guardError) {
      error = guardError;
      cancel();
      return;
    }
    loading = true;
    error = '';
    return async ({ result }: { result: ActionResult }) => {
      loading = false;
      if (result.type === 'failure') {
        const code = (result.data as { error?: string })?.error ?? '';
        error = failureMessage?.(code) ?? 'An error occurred. Please try again.';
        turnstileToken = '';
        turnstileRef?.reset();
      } else {
        done = resultIsSuccess ? resultIsSuccess(result) : true;
      }
    };
  };
</script>

{#snippet backToLogin()}
  <a
    href="/login"
    class="inline-flex items-center gap-1.5 font-bold text-brand-blue transition-opacity hover:opacity-60"
  >
    <svg
      xmlns="http://www.w3.org/2000/svg"
      width="8"
      height="12"
      viewBox="0 0 8 12"
      style="transform: rotate(180deg)"
    >
      <g fill="none" fill-rule="evenodd">
        <path fill="#0269FB" fill-rule="nonzero" d="M.59 10.59L5.17 6 .59 1.41 2 0l6 6-6 6z" />
      </g>
    </svg>
    Back to Log In
  </a>
{/snippet}

<svelte:head>
  <title>{title}</title>
</svelte:head>

<div class="min-h-[calc(100vh-120px)] bg-brand-light-blue -mx-6 -mt-8 px-6 py-16">
  <div class="mx-auto w-full max-w-[570px] bg-white px-[100px] py-16 max-md:px-8 max-md:py-10">
    {#if done}
      <div class="py-4 text-center">
        <h2 class="mb-4 font-heading text-2xl font-bold text-brand-text">{successHeading}</h2>
        <p class="text-base text-brand-muted">{successText}</p>
        {#if successBackLink}
          <div class="mt-8">
            {@render backToLogin()}
          </div>
        {/if}
      </div>
    {:else}
      <h2 class="mb-[34px] text-center font-heading text-2xl font-bold text-brand-text">
        {heading}
      </h2>

      {#if description}
        <p class="mb-6 text-sm text-brand-muted">{description}</p>
      {/if}

      <form method="POST" use:enhance={handleSubmit} class="space-y-4">
        {@render fields()}

        {#if turnstile.enabled && turnstile.sitekey}
          <Turnstile bind:this={turnstileRef} sitekey={turnstile.sitekey} ontoken={(t) => (turnstileToken = t)} />
          <input type="hidden" name="turnstile_token" value={turnstileToken} data-testid="cf-turnstile-token" />
        {/if}

        {#if error}
          <p class="text-sm text-brand-red">{error}</p>
        {/if}

        <button
          type="submit"
          disabled={loading || captchaPending}
          class="mt-2 inline-flex w-full items-center justify-center rounded-btn border-2 border-brand-blue bg-brand-blue px-5 py-[10px] font-heading text-base font-bold text-white transition-colors hover:bg-transparent hover:text-brand-blue disabled:opacity-50"
        >
          {loading ? loadingLabel : captchaPending ? 'Verifying…' : submitLabel}
        </button>
      </form>

      <div class="mt-8 text-center">
        {@render backToLogin()}
      </div>
    {/if}
  </div>
</div>
