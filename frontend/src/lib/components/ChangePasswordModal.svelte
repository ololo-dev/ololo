<script lang="ts">
  import { enhance } from '$app/forms';
  import Modal from './ui/Modal.svelte';
  import FormField from './ui/FormField.svelte';

  interface Props {
    open?: boolean;
    onClose?: () => void;
    onSuccess?: () => void;
    error?: string;
    loading?: boolean;
    action?: string;
  }

  let {
    open = false,
    onClose,
    onSuccess,
    error,
    loading = false,
    action = '/settings?/changePassword',
  }: Props = $props();

  let saving = $state(false);
  let showNewPassword = $state(false);
  let newPassword = $state('');
  let confirmPassword = $state('');
  let succeeded = $state(false);

  const passwordMismatch = $derived(
    confirmPassword.length > 0 && newPassword !== confirmPassword,
  );

  // Reset all form state when the modal closes so it reopens fresh
  $effect(() => {
    if (!open) {
      saving = false;
      showNewPassword = false;
      newPassword = '';
      confirmPassword = '';
      succeeded = false;
    }
  });
</script>

<Modal {open} {onClose}>
  {#if succeeded}
    <!-- Success state -->
    <div class="flex flex-col items-center py-4 text-center">
      <svg xmlns="http://www.w3.org/2000/svg" width="80" height="80" viewBox="0 0 80 80" fill="none">
        <circle cx="40" cy="40" r="37" stroke="#0269fb" stroke-width="3" />
        <polyline
          points="22,40 35,53 58,27"
          fill="none"
          stroke="#0269fb"
          stroke-width="4"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
      </svg>
      <p class="mb-8 mt-6 text-base text-brand-text">Your password was successfully changed.</p>
      <button
        type="button"
        class="inline-flex w-full items-center justify-center rounded-btn border-2 border-brand-blue bg-brand-blue px-5 py-[10px] font-heading text-base font-bold text-white transition-colors hover:bg-transparent hover:text-brand-blue"
        onclick={() => onClose?.()}
      >
        ok
      </button>
    </div>
  {:else}
    <form
      method="POST"
      {action}
      use:enhance={() => {
        saving = true;
        return async ({ result, update }) => {
          saving = false;
          if (result.type === 'success' && (result.data as { success?: boolean })?.success) {
            onSuccess?.();
            succeeded = true;
          }
          await update({ reset: false });
        };
      }}
    >
      <h2 class="mb-[34px] text-center font-heading text-2xl font-bold text-brand-text">
        Change password
      </h2>

      <div class="space-y-4">
        <FormField
          label="Current password"
          type="password"
          name="current_password"
          placeholder="Enter your current password"
          required
        />

        <!-- New password with show/hide toggle -->
        <div class="space-y-1">
          <div class="flex items-center justify-between">
            <label
              for="new-password"
              class="block cursor-pointer select-none text-xs font-semibold leading-snug text-brand-text"
            >
              New password <span class="ml-0.5 text-brand-red">*</span>
            </label>
            <button
              type="button"
              class="inline-flex items-center gap-2 bg-transparent text-sm font-semibold text-brand-muted transition-opacity hover:opacity-60"
              onclick={() => (showNewPassword = !showNewPassword)}
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
              {showNewPassword ? 'Hide password' : 'Show password'}
            </button>
          </div>
          <input
            id="new-password"
            name="new_password"
            type={showNewPassword ? 'text' : 'password'}
            placeholder="Enter your new password"
            required
            minlength="8"
            bind:value={newPassword}
            class="block h-12 w-full rounded-btn border-2 border-brand-border bg-white px-4 py-[5px] font-body text-base text-brand-text placeholder-brand-muted transition-colors focus:border-brand-blue focus:outline-none"
          />
          <p class="mt-0.5 block text-xs leading-none text-brand-muted">Minimum 8 characters</p>
        </div>

        <!-- Confirm new password -->
        <div class="space-y-1">
          <label
            for="confirm-password"
            class="block cursor-pointer select-none text-xs font-semibold leading-snug text-brand-text"
          >
            Confirm new password <span class="ml-0.5 text-brand-red">*</span>
          </label>
          <input
            id="confirm-password"
            name="confirm_password"
            type="password"
            placeholder="Enter your new password again"
            required
            bind:value={confirmPassword}
            class="block h-12 w-full rounded-btn border-2 border-brand-border bg-white px-4 py-[5px] font-body text-base text-brand-text placeholder-brand-muted transition-colors focus:border-brand-blue focus:outline-none"
          />
          {#if passwordMismatch}
            <p class="mt-0.5 block text-xs leading-none text-brand-red">Passwords do not match</p>
          {/if}
        </div>

        {#if error}
          <p class="text-sm text-brand-red">{error}</p>
        {/if}

        <button
          type="submit"
          disabled={saving || loading || passwordMismatch}
          class="mt-2 inline-flex w-full items-center justify-center rounded-btn border-2 border-brand-blue bg-brand-blue px-5 py-[10px] font-heading text-base font-bold text-white transition-colors hover:bg-transparent hover:text-brand-blue disabled:opacity-50"
        >
          {saving ? 'Changing…' : 'Change'}
        </button>
      </div>
    </form>
  {/if}
</Modal>
