<script lang="ts">
  import { enhance } from '$app/forms';

  let { data, form } = $props();

  let loading = $state(false);

  const errorMessages: Record<string, string> = {
    missing_fields: 'Please fill in all required fields.',
    login_failed: 'Invalid email or password.',
    session_expired: 'This login session has expired. Please run ololo login again.',
    confirm_failed: 'Something went wrong. Please try again.',
    not_authenticated: 'You must be logged in to authorize CLI access.',
  };

  const displayError = $derived(
    form?.error ? (errorMessages[form.error] ?? form.error) : ''
  );
</script>

<svelte:head>
  <title>ololo CLI Login</title>
</svelte:head>

<div class="-mx-6 -my-8 min-h-screen bg-brand-light-blue">
  <div class="mx-auto flex max-w-7xl px-6 py-16">
    <div class="mx-auto w-full max-w-[570px] bg-white px-[100px] py-16 max-md:px-8 max-md:py-10">

      {#if form?.confirmed}
        <!-- State 3: confirmed -->
        <h2 class="mb-4 text-center font-heading text-2xl font-bold text-brand-text">
          Login complete
        </h2>
        <p class="text-center text-brand-muted">Return to your terminal.</p>

      {:else if data.authenticated}
        <!-- State 2: logged in, needs confirm -->
        <h2 class="mb-[34px] text-center font-heading text-2xl font-bold text-brand-text">
          Authorize CLI access
        </h2>
        <p class="mb-6 text-center text-sm text-brand-muted">
          Click below to grant your terminal access to your ololo.dev account.
        </p>

        {#if displayError}
          <p class="mb-4 text-sm text-brand-red">{displayError}</p>
        {/if}

        <form
          method="POST"
          action="?/confirm&cli_token={encodeURIComponent(data.cliToken)}"
          use:enhance={() => {
            loading = true;
            return async ({ update }) => {
              loading = false;
              await update();
            };
          }}
        >
          <button
            type="submit"
            disabled={loading}
            class="inline-flex w-full items-center justify-center rounded-btn border-2 border-brand-blue bg-brand-blue px-5 py-[10px] font-heading text-base font-bold text-white transition-colors hover:bg-transparent hover:text-brand-blue disabled:opacity-50"
          >
            {loading ? 'Authorizing…' : 'Authorize CLI access'}
          </button>
        </form>

      {:else}
        <!-- State 1: not logged in -->
        <h2 class="mb-[34px] text-center font-heading text-2xl font-bold text-brand-text">
          Log in to ololo CLI
        </h2>

        <form
          method="POST"
          action="?/login&cli_token={encodeURIComponent(data.cliToken)}"
          use:enhance={() => {
            loading = true;
            return async ({ result, update }) => {
              loading = false;
              await update();
            };
          }}
        >
          <div class="space-y-4">
            <div class="space-y-1">
              <label
                for="email"
                class="block cursor-pointer text-xs font-semibold leading-snug text-brand-text select-none"
              >
                Your email
              </label>
              <input
                id="email"
                name="email"
                type="email"
                autocomplete="email"
                placeholder="Enter your email"
                value={form?.email ?? ''}
                required
                class="block h-12 w-full rounded-btn border-2 border-brand-border bg-white px-4 py-[5px] font-body text-base text-brand-text placeholder-brand-muted transition-colors focus:border-brand-blue focus:outline-none"
              />
            </div>

            <div class="space-y-1">
              <label
                for="password"
                class="block cursor-pointer text-xs font-semibold leading-snug text-brand-text select-none"
              >
                Your password
              </label>
              <input
                id="password"
                name="password"
                type="password"
                placeholder="Enter your password"
                required
                class="block h-12 w-full rounded-btn border-2 border-brand-border bg-white px-4 py-[5px] font-body text-base text-brand-text placeholder-brand-muted transition-colors focus:border-brand-blue focus:outline-none"
              />
            </div>

            {#if displayError}
              <p class="text-sm text-brand-red">{displayError}</p>
            {/if}

            <button
              type="submit"
              disabled={loading}
              class="mt-2 inline-flex w-full items-center justify-center rounded-btn border-2 border-brand-blue bg-brand-blue px-5 py-[10px] font-heading text-base font-bold text-white transition-colors hover:bg-transparent hover:text-brand-blue disabled:opacity-50"
            >
              {loading ? 'Logging in…' : 'Log in'}
            </button>
          </div>
        </form>
      {/if}

    </div>
  </div>
</div>
