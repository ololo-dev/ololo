<script lang="ts">
  import { page } from '$app/stores';
  import AuthFormPage from '$lib/components/auth/AuthFormPage.svelte';

  let { data } = $props();

  let token = $derived($page.url.searchParams.get('token') ?? '');
  let newPassword = $state('');
</script>

<AuthFormPage
  title="Reset password — ololo.dev"
  heading="Set new password"
  submitLabel="Reset Password"
  loadingLabel="Resetting…"
  turnstile={data.turnstile}
  successHeading="Password reset!"
  successText="Your password has been updated. Redirecting to login…"
  guard={() => (token ? null : 'Invalid reset link.')}
  failureMessage={(code) => code || 'Reset failed. The link may be expired or invalid.'}
  resultIsSuccess={(result) =>
    result.type === 'success' && !!(result.data as { success?: boolean })?.success}
>
  {#snippet fields()}
    <input type="hidden" name="token" value={token} />
    <div class="space-y-1">
      <label
        for="new-password"
        class="block cursor-pointer text-xs font-semibold leading-snug text-brand-text select-none"
      >
        New password
      </label>
      <input
        id="new-password"
        type="password"
        name="new_password"
        required
        minlength="8"
        value={newPassword}
        placeholder="Enter new password"
        oninput={(e) => { newPassword = (e.target as HTMLInputElement).value; }}
        class="block h-12 w-full rounded-btn border-2 border-brand-border bg-white px-4 py-[5px] font-body text-base text-brand-text placeholder-brand-muted transition-colors focus:border-brand-blue focus:outline-none"
      />
      <p class="mt-0.5 block text-xs leading-none text-brand-muted">Minimum 8 symbols</p>
    </div>
  {/snippet}
</AuthFormPage>
