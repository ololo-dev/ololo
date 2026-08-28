<script lang="ts">
  import Modal from './ui/Modal.svelte';
  import PasswordStrengthMeter from './ui/PasswordStrengthMeter.svelte';
  import Turnstile from './Turnstile.svelte';
  import { enhance } from '$app/forms';
  import { invalidate } from '$app/navigation';
  import { untrack } from 'svelte';
  import { browser } from '$app/environment';
  import { getTurnstileConfig, type TurnstileConfigResponse } from '$lib/api';

  interface Props {
    open?: boolean;
    mode?: 'login' | 'register';
    onclose?: () => void;
    initialError?: string;
  }

  let { open = false, mode = 'login', onclose, initialError = '' }: Props = $props();

  type ModalMode = 'login' | 'register' | 'forgot' | 'magic';
  let currentMode = $state<ModalMode>(untrack(() => mode ?? 'login'));
  let loginError = $state('');
  let registerError = $state('');
  let loginLoading = $state(false);
  let registerLoading = $state(false);
  let emailFlowError = $state('');
  let emailFlowLoading = $state(false);
  let emailFlowSuccess = $state(false);
  let emailFlowTurnstileToken = $state('');
  let showPassword = $state(false);
  // Sign-up keeps its own toggle: it reveals two fields at once, and carrying
  // a reveal across a mode switch would surprise whoever typed the other one.
  let showRegisterPassword = $state(false);
  let registerPassword = $state('');
  let registerRepeatPassword = $state('');
  let loginTurnstileToken = $state('');
  let registerTurnstileToken = $state('');
  let turnstileCfg = $state<TurnstileConfigResponse | null>(null);
  let turnstileCfgLoading = $state(false);
  // Only one mode's form (and therefore one widget) is mounted at a time.
  let turnstileRef = $state<{ reset: () => void } | null>(null);

  const turnstileRequired = $derived(!!(turnstileCfg?.enabled && turnstileCfg?.sitekey));
  // Block submits until the widget has produced a token — otherwise a fast
  // user posts an empty token and gets a captcha_required error back.
  const loginCaptchaPending = $derived(
    turnstileCfgLoading || (turnstileRequired && !loginTurnstileToken)
  );
  const registerCaptchaPending = $derived(
    turnstileCfgLoading || (turnstileRequired && !registerTurnstileToken)
  );
  const emailFlowCaptchaPending = $derived(
    turnstileCfgLoading || (turnstileRequired && !emailFlowTurnstileToken)
  );

  const registerErrorMessages: Record<string, string> = {
    missing_fields: 'Please fill in all required fields.',
    user_exists: 'An account with this email address already exists.',
    register_failed: 'Registration failed. Please try again.',
    database_error: 'A database error occurred. Please try again later.',
    internal_error: 'An unexpected error occurred. Please try again.',
    hash_error: 'An unexpected error occurred. Please try again.',
    invalid_credentials: 'Invalid email or password.',
    captcha_required: 'CAPTCHA verification is required. Please complete it and try again.',
    captcha_invalid: 'CAPTCHA verification failed. Please try again.',
    captcha_error: 'CAPTCHA service is unavailable. Please try again later.',
  };

  const displayRegisterError = $derived(
    registerError ? (registerErrorMessages[registerError] ?? registerError) : ''
  );

  const loginErrorMessages: Record<string, string> = {
    login_failed: 'Invalid email or password.',
    captcha_required: 'CAPTCHA verification is required. Please complete it and try again.',
    captcha_invalid: 'CAPTCHA verification failed. Please try again.',
    captcha_error: 'CAPTCHA service is unavailable. Please try again later.',
  };

  const displayLoginError = $derived(
    loginError ? (loginErrorMessages[loginError] ?? loginError) : ''
  );

  $effect(() => {
    if (open) {
      currentMode = mode ?? 'login';
      loginError = untrack(() => initialError ?? '');
      registerError = '';
      loginLoading = false;
      registerLoading = false;
      registerPassword = '';
      registerRepeatPassword = '';
      showRegisterPassword = false;
      loginTurnstileToken = '';
      registerTurnstileToken = '';
      emailFlowError = '';
      emailFlowLoading = false;
      emailFlowSuccess = false;
      emailFlowTurnstileToken = '';
      // Fetch Turnstile config if not already loaded.
      if (browser && !turnstileCfg) {
        turnstileCfgLoading = true;
        getTurnstileConfig()
          .then((cfg) => { turnstileCfg = cfg; })
          .catch(() => {})
          .finally(() => { turnstileCfgLoading = false; });
      }
    }
  });

  const loginEnhance = () => {
    loginLoading = true;
    loginError = '';
    return async ({ result }: { result: import('@sveltejs/kit').ActionResult }) => {
      loginLoading = false;
      if (result.type === 'failure') {
        loginError = (result.data as { error?: string })?.error ?? 'login_failed';
        loginTurnstileToken = '';
        turnstileRef?.reset();
      } else if (result.type === 'success' || result.type === 'redirect') {
        await invalidate('app:user');
        onclose?.();
      }
    };
  };

  const registerEnhance = () => {
    registerLoading = true;
    registerError = '';
    return async ({ result }: { result: import('@sveltejs/kit').ActionResult }) => {
      registerLoading = false;
      if (result.type === 'failure') {
        registerError = (result.data as { error?: string })?.error ?? 'register_failed';
        registerTurnstileToken = '';
        turnstileRef?.reset();
      } else if (result.type === 'success' || result.type === 'redirect') {
        await invalidate('app:user');
        onclose?.();
      }
    };
  };

  const emailFlowErrorMessages: Record<string, string> = {
    missing_fields: 'Please enter your email address.',
    captcha_error: 'CAPTCHA verification failed. Please try again.',
    rate_limited: 'Too many requests. Please wait a moment and try again.',
  };
  const displayEmailFlowError = $derived(
    emailFlowError ? (emailFlowErrorMessages[emailFlowError] ?? emailFlowError) : ''
  );

  // Shared by the forgot-password and magic-link forms (only one is mounted
  // at a time). Posts to the standalone pages' form actions.
  const emailFlowEnhance = () => {
    emailFlowLoading = true;
    emailFlowError = '';
    return async ({ result }: { result: import('@sveltejs/kit').ActionResult }) => {
      emailFlowLoading = false;
      if (result.type === 'failure') {
        emailFlowError = (result.data as { error?: string })?.error ?? 'captcha_error';
        emailFlowTurnstileToken = '';
        turnstileRef?.reset();
      } else if (result.type === 'success') {
        emailFlowSuccess = true;
      }
    };
  };

  const switchMode = (m: ModalMode) => {
    currentMode = m;
    emailFlowError = '';
    emailFlowLoading = false;
    emailFlowSuccess = false;
    emailFlowTurnstileToken = '';
  };

  const modalLabels: Record<ModalMode, string> = {
    login: 'Log in',
    register: 'Create account',
    forgot: 'Reset password',
    magic: 'Log in with magic link',
  };
</script>

{#snippet magicLinkOption(dividerLabel: string)}
  <!-- The alternative sign-in block on the login view: a one-time
       emailed link instead of the password. -->
        <div class="relative my-8 text-center text-base font-semibold leading-normal text-brand-muted">
          <div class="absolute inset-0 top-1/2 h-0.5 bg-brand-border"></div>
          <span class="relative z-10 bg-white px-4">{dividerLabel}</span>
        </div>

        <div class="-mx-4 flex flex-wrap gap-y-2">
          <div class="w-full px-4">
            <button
              type="button"
              onclick={() => switchMode('magic')}
              class="flex w-full items-center justify-center gap-3 rounded-btn border-2 border-brand-border px-4 py-[10px] font-heading text-base font-bold text-brand-text transition-colors hover:border-brand-blue hover:text-brand-blue bg-transparent"
            >
              <svg class="h-6 w-6" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none">
                <path d="M4 6h16v12H4z" stroke="currentColor" stroke-width="2" stroke-linejoin="round" />
                <path d="m4 7 8 6 8-6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
              </svg>
              Magic link
            </button>
          </div>
        </div>
{/snippet}


<Modal {open} onClose={onclose}>
  <div role="dialog" aria-modal="true" aria-label={modalLabels[currentMode]}>
    {#if currentMode === 'login'}
      <form action="/login" method="POST" use:enhance={loginEnhance}>
        <h2 class="mb-[34px] text-center font-heading text-2xl font-bold text-brand-text">
          Log in
        </h2>

        <div class="space-y-4">
          <div class="space-y-1">
            <label
              for="auth-login-email"
              class="block cursor-pointer text-xs font-semibold leading-snug text-brand-text select-none"
            >
              Your email
            </label>
            <input
              id="auth-login-email"
              name="email"
              type="email"
              autocomplete="email"
              placeholder="Enter your email"
              required
              class="block h-12 w-full rounded-btn border-2 border-brand-border bg-white px-4 py-[5px] font-body text-base text-brand-text placeholder-brand-muted transition-colors focus:border-brand-blue focus:outline-none"
            />
          </div>

          <div>
            <div class="flex items-center justify-between">
              <label
                for="auth-login-password"
                class="block cursor-pointer text-xs font-semibold leading-snug text-brand-text select-none"
              >
                Your password
              </label>
            </div>
            <div class="relative mt-1">
              <input
                id="auth-login-password"
                name="password"
                type={showPassword ? 'text' : 'password'}
                placeholder="Enter your password"
                required
                class="block h-12 w-full rounded-btn border-2 border-brand-border bg-white px-4 py-[5px] font-body text-base text-brand-text placeholder-brand-muted transition-colors focus:border-brand-blue focus:outline-none"
              />
            </div>

            <div class="mt-2 flex items-center justify-between text-sm font-semibold leading-[1.71]">
              <button
                type="button"
                class="text-brand-blue transition-opacity hover:opacity-60 bg-transparent"
                onclick={() => switchMode('forgot')}
              >
                Forgot password
              </button>
              <button
                type="button"
                class="inline-flex items-center gap-2 text-brand-muted transition-opacity hover:opacity-60 bg-transparent"
                onclick={() => (showPassword = !showPassword)}
              >
                <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24">
                  <g fill="none" fill-rule="evenodd">
                    <path d="M0 0h24v24H0z" />
                    <path
                      fill="#9EA7B6"
                      fill-rule="nonzero"
                      d="M12 6c-4.09 0-7.585 2.488-9 6 1.415 3.512 4.91 6 9 6s7.585-2.488 9-6c-1.415-3.512-4.91-6-9-6zm0 10c-2.258 0-4.09-1.792-4.09-4S9.741 8 12 8s4.09 1.792 4.09 4-1.832 4-4.09 4zm0-6.4c-1.358 0-2.455 1.072-2.455 2.4 0 1.328 1.097 2.4 2.455 2.4s2.455-1.072 2.455-2.4c0-1.328-1.097-2.4-2.455-2.4z"
                    />
                  </g>
                </svg>
                {showPassword ? 'Hide password' : 'Show password'}
              </button>
            </div>
          </div>

          {#if loginError}
            <p class="text-sm text-brand-red" data-testid="login-modal-error">{displayLoginError}</p>
          {/if}

          {#if turnstileCfg?.enabled && turnstileCfg?.sitekey}
            <Turnstile bind:this={turnstileRef} sitekey={turnstileCfg.sitekey} ontoken={(t) => (loginTurnstileToken = t)} />
            <input type="hidden" name="turnstile_token" value={loginTurnstileToken} data-testid="cf-turnstile-token" />
          {/if}

          <button
            type="submit"
            disabled={loginLoading || loginCaptchaPending}
            class="mt-2 inline-flex w-full items-center justify-center rounded-btn border-2 border-brand-blue bg-brand-blue px-5 py-[10px] font-heading text-base font-bold text-white transition-colors hover:bg-transparent hover:text-brand-blue disabled:opacity-50"
          >
            {loginLoading ? 'Logging in…' : loginCaptchaPending ? 'Verifying…' : 'Log in'}
          </button>
        </div>

        {@render magicLinkOption('or log in with')}

        <div class="mt-12 text-center text-base">
          Don't have an account?
          <button
            type="button"
            class="ml-2 inline-flex items-center gap-1.5 font-bold text-brand-blue transition-opacity hover:opacity-60 bg-transparent"
            onclick={() => (currentMode = 'register')}
          >
            Sign Up
            <svg xmlns="http://www.w3.org/2000/svg" width="8" height="12" viewBox="0 0 8 12">
              <g fill="none" fill-rule="evenodd">
                <path fill="#0269FB" fill-rule="nonzero" d="M.59 10.59L5.17 6 .59 1.41 2 0l6 6-6 6z" />
              </g>
            </svg>
          </button>
        </div>
      </form>
    {:else if currentMode === 'register'}
      <form action="/register" method="POST" use:enhance={registerEnhance}>
        <h2 class="mb-[34px] text-center font-heading text-2xl font-bold text-brand-text">
          Create new account
        </h2>

        <div class="space-y-4">
          <div class="space-y-1">
            <label
              for="auth-reg-email"
              class="block cursor-pointer text-xs font-semibold leading-snug text-brand-text select-none"
            >
              Your e-mail
            </label>
            <input
              id="auth-reg-email"
              name="email"
              type="email"
              required
              placeholder="Enter your email"
              class="block h-12 w-full rounded-btn border-2 border-brand-border bg-white px-4 font-body text-base text-brand-text placeholder-brand-muted transition-colors focus:border-brand-blue focus:outline-none"
            />
          </div>

          <div class="space-y-1">
            <div class="mb-1 text-xs font-semibold leading-snug text-brand-text select-none">Name</div>
            <input
              type="text"
              name="display_name"
              placeholder="Display name"
              required
              class="block h-12 w-full rounded-btn border-2 border-brand-border bg-white px-4 font-body text-base text-brand-text placeholder-brand-muted transition-colors focus:border-brand-blue focus:outline-none"
            />
          </div>

          <div class="space-y-1">
            <label
              for="auth-reg-password"
              class="block cursor-pointer text-xs font-semibold leading-snug text-brand-text select-none"
            >
              Your password
            </label>
            <input
              id="auth-reg-password"
              name="password"
              type={showRegisterPassword ? 'text' : 'password'}
              required
              minlength="8"
              autocomplete="new-password"
              aria-describedby="auth-reg-password-strength"
              placeholder="Enter your password"
              oninput={(e) => (registerPassword = e.currentTarget.value)}
              class="block h-12 w-full rounded-btn border-2 border-brand-border bg-white px-4 font-body text-base text-brand-text placeholder-brand-muted transition-colors focus:border-brand-blue focus:outline-none {registerPassword.length > 0 && registerPassword.length < 8 ? 'border-brand-red' : ''}"
            />
            {#if registerPassword.length > 0 && registerPassword.length < 8}
              <p class="mt-0.5 block text-xs leading-none text-brand-red">
                Password must include at least 8 symbols. Please, correct
              </p>
            {:else}
              <p class="mt-0.5 block text-xs leading-none text-brand-muted">Minimum 8 symbols</p>
            {/if}

            <PasswordStrengthMeter password={registerPassword} id="auth-reg-password-strength" />

            <div class="mt-2 flex justify-end text-sm font-semibold leading-[1.71]">
              <button
                type="button"
                data-testid="register-toggle-password"
                aria-pressed={showRegisterPassword}
                class="inline-flex items-center gap-2 text-brand-muted transition-opacity hover:opacity-60 bg-transparent"
                onclick={() => (showRegisterPassword = !showRegisterPassword)}
              >
                <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24">
                  <g fill="none" fill-rule="evenodd">
                    <path d="M0 0h24v24H0z" />
                    <path
                      fill="#9EA7B6"
                      fill-rule="nonzero"
                      d="M12 6c-4.09 0-7.585 2.488-9 6 1.415 3.512 4.91 6 9 6s7.585-2.488 9-6c-1.415-3.512-4.91-6-9-6zm0 10c-2.258 0-4.09-1.792-4.09-4S9.741 8 12 8s4.09 1.792 4.09 4-1.832 4-4.09 4zm0-6.4c-1.358 0-2.455 1.072-2.455 2.4 0 1.328 1.097 2.4 2.455 2.4s2.455-1.072 2.455-2.4c0-1.328-1.097-2.4-2.455-2.4z"
                    />
                  </g>
                </svg>
                {showRegisterPassword ? 'Hide password' : 'Show password'}
              </button>
            </div>
          </div>

          <div class="space-y-1">
            <label
              for="auth-reg-repeat-password"
              class="block cursor-pointer text-xs font-semibold leading-snug text-brand-text select-none"
            >
              Repeat password
            </label>
            <input
              id="auth-reg-repeat-password"
              name="repeat_password"
              type={showRegisterPassword ? 'text' : 'password'}
              required
              autocomplete="new-password"
              placeholder="Repeat your password"
              oninput={(e) => (registerRepeatPassword = e.currentTarget.value)}
              class="block h-12 w-full rounded-btn border-2 border-brand-border bg-white px-4 font-body text-base text-brand-text placeholder-brand-muted transition-colors focus:border-brand-blue focus:outline-none {registerRepeatPassword.length > 0 && registerRepeatPassword !== registerPassword ? 'border-brand-red' : ''}"
            />
            {#if registerRepeatPassword.length > 0 && registerRepeatPassword !== registerPassword}
              <p class="mt-0.5 block text-xs leading-none text-brand-red">
                Passwords do not match
              </p>
            {/if}
          </div>

          {#if displayRegisterError}
            <p class="text-sm text-brand-red" data-testid="register-modal-error">{displayRegisterError}</p>
          {/if}

          {#if turnstileCfg?.enabled && turnstileCfg?.sitekey}
            <Turnstile bind:this={turnstileRef} sitekey={turnstileCfg.sitekey} ontoken={(t) => (registerTurnstileToken = t)} />
            <input type="hidden" name="turnstile_token" value={registerTurnstileToken} data-testid="cf-turnstile-token" />
          {/if}

          <button
            type="submit"
            disabled={registerLoading || registerCaptchaPending}
            class="mt-2 inline-flex w-full items-center justify-center rounded-btn border-2 border-brand-blue bg-brand-blue px-5 py-[10px] font-heading text-base font-bold text-white transition-colors hover:bg-transparent hover:text-brand-blue disabled:opacity-50"
          >
            {registerLoading ? 'Registering…' : registerCaptchaPending ? 'Verifying…' : 'Register'}
          </button>

          <p class="text-center text-xs leading-snug text-brand-muted">
            By creating an account you agree to the
            <a href="/terms" target="_blank" class="underline hover:text-brand-blue">Terms of Service</a>
            and
            <a href="/privacy" target="_blank" class="underline hover:text-brand-blue">Privacy Policy</a>.
          </p>
        </div>

        <div class="mt-12 text-center text-base">
          Already registered?
          <button
            type="button"
            class="ml-2 inline-flex items-center gap-1.5 font-bold text-brand-blue transition-opacity hover:opacity-60 bg-transparent"
            onclick={() => (currentMode = 'login')}
          >
            Log In
            <svg xmlns="http://www.w3.org/2000/svg" width="8" height="12" viewBox="0 0 8 12">
              <g fill="none" fill-rule="evenodd">
                <path fill="#0269FB" fill-rule="nonzero" d="M.59 10.59L5.17 6 .59 1.41 2 0l6 6-6 6z" />
              </g>
            </svg>
          </button>
        </div>
      </form>
    {:else}
      {@const isForgot = currentMode === 'forgot'}
      {#if emailFlowSuccess}
        <div class="py-4 text-center">
          <h2 class="mb-4 font-heading text-2xl font-bold text-brand-text">Check your inbox</h2>
          <p class="text-base text-brand-muted">
            {isForgot
              ? 'If an account with that address exists, a password reset link is on its way.'
              : 'If an account with that address exists, a sign-in link is on its way.'}
          </p>
          <button
            type="button"
            class="mt-8 inline-flex items-center gap-1.5 font-bold text-brand-blue transition-opacity hover:opacity-60 bg-transparent"
            onclick={() => switchMode('login')}
          >
            Back to log in
          </button>
        </div>
      {:else}
        <form
          action={isForgot ? '/forgot-password' : '/magic-link'}
          method="POST"
          use:enhance={emailFlowEnhance}
        >
          <h2 class="mb-4 text-center font-heading text-2xl font-bold text-brand-text">
            {isForgot ? 'Reset password' : 'Log in with magic link'}
          </h2>
          <p class="mb-[34px] text-center text-base text-brand-muted">
            {isForgot
              ? 'Enter your email and we will send you a link to reset your password.'
              : 'Enter your email and we will send you a one-time sign-in link.'}
          </p>

          <div class="space-y-4">
            <div class="space-y-1">
              <label
                for="auth-email-flow-email"
                class="block cursor-pointer text-xs font-semibold leading-snug text-brand-text select-none"
              >
                Your email
              </label>
              <input
                id="auth-email-flow-email"
                name="email"
                type="email"
                autocomplete="email"
                placeholder="Enter your email"
                required
                class="block h-12 w-full rounded-btn border-2 border-brand-border bg-white px-4 py-[5px] font-body text-base text-brand-text placeholder-brand-muted transition-colors focus:border-brand-blue focus:outline-none"
              />
            </div>

            {#if displayEmailFlowError}
              <p class="text-sm text-brand-red" data-testid="email-flow-modal-error">{displayEmailFlowError}</p>
            {/if}

            {#if turnstileCfg?.enabled && turnstileCfg?.sitekey}
              <Turnstile bind:this={turnstileRef} sitekey={turnstileCfg.sitekey} ontoken={(t) => (emailFlowTurnstileToken = t)} />
              <input type="hidden" name="turnstile_token" value={emailFlowTurnstileToken} data-testid="cf-turnstile-token" />
            {/if}

            <button
              type="submit"
              disabled={emailFlowLoading || emailFlowCaptchaPending}
              class="mt-2 inline-flex w-full items-center justify-center rounded-btn border-2 border-brand-blue bg-brand-blue px-5 py-[10px] font-heading text-base font-bold text-white transition-colors hover:bg-transparent hover:text-brand-blue disabled:opacity-50"
            >
              {emailFlowLoading
                ? 'Sending…'
                : emailFlowCaptchaPending
                  ? 'Verifying…'
                  : isForgot
                    ? 'Send reset link'
                    : 'Send magic link'}
            </button>
          </div>

          <div class="mt-12 text-center text-base">
            Remembered your password?
            <button
              type="button"
              class="ml-2 inline-flex items-center gap-1.5 font-bold text-brand-blue transition-opacity hover:opacity-60 bg-transparent"
              onclick={() => switchMode('login')}
            >
              Log In
              <svg xmlns="http://www.w3.org/2000/svg" width="8" height="12" viewBox="0 0 8 12">
                <g fill="none" fill-rule="evenodd">
                  <path fill="#0269FB" fill-rule="nonzero" d="M.59 10.59L5.17 6 .59 1.41 2 0l6 6-6 6z" />
                </g>
              </svg>
            </button>
          </div>
        </form>
      {/if}
    {/if}
  </div>
</Modal>
