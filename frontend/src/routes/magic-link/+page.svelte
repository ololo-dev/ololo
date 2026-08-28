<script lang="ts">
  import AuthFormPage from '$lib/components/auth/AuthFormPage.svelte';

  let { data } = $props();

  let email = $state('');

  const captchaErrors: Record<string, string> = {
    captcha_required: 'CAPTCHA verification is required. Please complete it and try again.',
    captcha_invalid: 'CAPTCHA verification failed. Please try again.',
    captcha_error: 'CAPTCHA service is unavailable. Please try again later.',
  };
</script>

<AuthFormPage
  title="Log in — ololo.dev"
  heading="Sign in with magic link"
  description="Enter your email address and we'll send you a magic link to sign in instantly — no password required."
  submitLabel="Send Magic Link"
  loadingLabel="Sending…"
  turnstile={data.turnstile}
  successHeading="Check your email"
  successText="If an account with that address exists, a magic link has been sent. Check your inbox."
  successBackLink
  failureMessage={(code) => captchaErrors[code] ?? 'An error occurred. Please try again.'}
>
  {#snippet fields()}
    <div class="space-y-1">
      <label
        for="magic-email"
        class="block cursor-pointer text-xs font-semibold leading-snug text-brand-text select-none"
      >
        Your email
      </label>
      <input
        id="magic-email"
        type="email"
        name="email"
        required
        value={email}
        placeholder="Enter your email"
        oninput={(e) => { email = (e.target as HTMLInputElement).value; }}
        class="block h-12 w-full rounded-btn border-2 border-brand-border bg-white px-4 py-[5px] font-body text-base text-brand-text placeholder-brand-muted transition-colors focus:border-brand-blue focus:outline-none"
      />
    </div>
  {/snippet}
</AuthFormPage>
