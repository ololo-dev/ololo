<script lang="ts">
  import { invalidateAll } from '$app/navigation';
  import Modal from './ui/Modal.svelte';
  import FormField from './ui/FormField.svelte';
  import {
    createAdminUser,
    updateAdminUser,
    type AdminUserDto,
    ApiError,
  } from '$lib/api';
  import { notify } from '$lib/notifications.svelte';

  interface Props {
    open?: boolean;
    onClose?: () => void;
    /** Provide to enter edit mode; omit or null for create mode. */
    user?: AdminUserDto | null;
  }

  let { open = false, onClose, user = null }: Props = $props();

  let email = $state('');
  let displayName = $state('');
  let username = $state('');
  let password = $state('');
  let isAdmin = $state(false);
  /** Per-user monthly judge-run limit override; empty = the default limit applies. */
  let judgeRunLimit = $state('');
  /** Extra judge-run credits to grant on save (a delta; negative corrects). */
  let grantCredits = $state('');
  let saving = $state(false);
  let fieldError = $state<string | undefined>(undefined);

  // Reset form whenever the modal opens or the target user changes.
  $effect(() => {
    if (open) {
      email = user?.email ?? '';
      displayName = user?.display_name ?? '';
      username = user?.username ?? '';
      password = '';
      isAdmin = user?.is_admin ?? false;
      judgeRunLimit = user?.judge_run_limit != null ? String(user.judge_run_limit) : '';
      grantCredits = '';
      fieldError = undefined;
    }
  });

  const isEdit = $derived(user != null);

  async function handleSubmit(e: SubmitEvent) {
    e.preventDefault();
    if (saving) return;
    saving = true;
    fieldError = undefined;

    const trimmedLimit = judgeRunLimit.trim();
    const parsedLimit = trimmedLimit === '' ? null : Number(trimmedLimit);
    if (parsedLimit !== null && (!Number.isInteger(parsedLimit) || parsedLimit < 0)) {
      fieldError = 'Judge run limit must be a non-negative whole number, or blank for the default.';
      saving = false;
      return;
    }
    const trimmedGrant = grantCredits.trim();
    const parsedGrant = trimmedGrant === '' ? 0 : Number(trimmedGrant);
    if (!Number.isInteger(parsedGrant)) {
      fieldError = 'Credits grant must be a whole number (negative to correct).';
      saving = false;
      return;
    }

    try {
      if (isEdit && user) {
        const body: {
          email?: string; display_name?: string; username?: string; is_admin?: boolean;
          password?: string; judge_run_limit?: number | null;
          grant_judge_run_credits?: number;
        } = {};
        if (email !== user.email) body.email = email;
        if (displayName !== user.display_name) body.display_name = displayName;
        if (username !== (user.username ?? '')) body.username = username;
        if (isAdmin !== user.is_admin) body.is_admin = isAdmin;
        if (password) body.password = password;
        if (parsedLimit !== (user.judge_run_limit ?? null)) body.judge_run_limit = parsedLimit;
        if (parsedGrant !== 0) body.grant_judge_run_credits = parsedGrant;
        await updateAdminUser(user.id, body);
        notify.success('User updated successfully.', 'Users');
      } else {
        await createAdminUser({
          email,
          display_name: displayName,
          ...(username.trim().length >= 4 ? { username: username.trim() } : {}),
          password,
          is_admin: isAdmin,
        });
        notify.success('User created successfully.', 'Users');
      }
      await invalidateAll();
      onClose?.();
    } catch (err) {
      if (err instanceof ApiError && err.code === 'email_taken') {
        fieldError = 'That email address is already in use.';
      } else if (err instanceof ApiError && err.code === 'username_taken') {
        fieldError = 'That username is already taken.';
      } else if (err instanceof ApiError && err.code === 'invalid_username') {
        fieldError = 'Username must be 4–30 characters: lowercase letters, digits, hyphens, underscores; must start with a letter and end with a letter or digit.';
      } else {
        fieldError = 'An error occurred. Please try again.';
      }
    } finally {
      saving = false;
    }
  }
</script>

<Modal {open} {onClose}>
  <form onsubmit={handleSubmit}>
    <h2 class="mb-6 text-center font-heading text-2xl font-bold text-brand-text">
      {isEdit ? 'Edit User' : 'Add User'}
    </h2>

    <div class="space-y-4">
      <FormField
        label="Email"
        type="email"
        name="email"
        placeholder="user@example.com"
        value={email}
        required
        onchange={(v) => (email = v)}
      />

      <FormField
        label="Display name"
        type="text"
        name="display_name"
        placeholder="e.g. Jane Smith"
        value={displayName}
        required
        onchange={(v) => (displayName = v)}
      />

      {#if isEdit}
        <FormField
          label="Username"
          type="text"
          name="username"
          placeholder="e.g. jane-doe5"
          value={username}
          onchange={(v) => (username = v)}
        />
      {:else}
        <FormField
          label="Username (leave blank to auto-generate)"
          type="text"
          name="username"
          placeholder="e.g. jane-doe5"
          value={username}
          onchange={(v) => (username = v)}
        />
      {/if}

      <FormField
        label={isEdit ? 'New password (leave blank to keep current)' : 'Password'}
        type="password"
        name="password"
        placeholder={isEdit ? 'Leave blank to keep current' : 'Minimum 8 characters'}
        value={password}
        required={!isEdit}
        onchange={(v) => (password = v)}
      />

      <FormField
        label="Monthly judge run limit (leave blank for the default)"
        type="text"
        name="judge_run_limit"
        placeholder="e.g. 500"
        value={judgeRunLimit}
        onchange={(v) => (judgeRunLimit = v)}
      />

      {#if isEdit}
        <FormField
          label={`Grant extra judge runs (current balance: ${user?.judge_run_credits ?? 0})`}
          type="text"
          name="grant_judge_run_credits"
          placeholder="e.g. 1000 — adds to the balance; negative corrects"
          value={grantCredits}
          onchange={(v) => (grantCredits = v)}
        />
      {/if}

      <!-- Admin toggle -->
      <div>
        <div class="mb-1 text-xs font-semibold text-brand-text">Permissions</div>
        <label class="flex cursor-pointer items-center gap-3">
          <div class="relative">
            <input
              type="checkbox"
              class="sr-only"
              checked={isAdmin}
              oninput={(e) => (isAdmin = (e.target as HTMLInputElement).checked)}
            />
            <div
              class="h-6 w-11 rounded-full transition-colors duration-200"
              class:bg-brand-blue={isAdmin}
              class:bg-gray-300={!isAdmin}
            ></div>
            <div
              class="absolute left-1 top-1 h-4 w-4 rounded-full bg-white shadow transition-transform duration-200"
              style:transform={isAdmin ? 'translateX(20px)' : 'translateX(0)'}
            ></div>
          </div>
          <span class="text-sm text-brand-text">Admin privileges</span>
        </label>
      </div>

      {#if fieldError}
        <p class="text-sm text-brand-red">{fieldError}</p>
      {/if}

      <button
        type="submit"
        disabled={saving}
        class="mt-2 inline-flex w-full items-center justify-center rounded-btn border-2 border-brand-blue
               bg-brand-blue px-5 py-[10px] font-heading text-base font-bold text-white
               transition-colors hover:bg-transparent hover:text-brand-blue disabled:opacity-50"
      >
        {saving ? 'Saving…' : isEdit ? 'Save Changes' : 'Create User'}
      </button>
    </div>
  </form>
</Modal>
