<script lang="ts">
  import { ikAvatar } from '$lib/imagekit';
  import { invalidateAll } from '$app/navigation';
  import Modal from './ui/Modal.svelte';
  import FormField from './ui/FormField.svelte';
  import { getAvatarAuth, patchMe, ApiError } from '$lib/api';
  import { notify } from '$lib/notifications.svelte';
  import { IMAGE_ACCEPT_ATTR, ACCEPTED_IMAGE_LABEL, validateImageFile } from '$lib/image-upload';

  interface Props {
    open?: boolean;
    onClose?: () => void;
    onSuccess?: () => void;
    displayName?: string;
    email?: string;
    emailVerified?: boolean;
    userId?: string;
    avatarUrl?: string | null;
    error?: string;
    loading?: boolean;
  }

  let {
    open = false,
    onClose,
    onSuccess,
    displayName = '',
    email = '',
    emailVerified = false,
    userId = '',
    avatarUrl = null,
    error,
    loading = false,
  }: Props = $props();

  let saving = $state(false);
  let uploadError = $state<string | undefined>(undefined);
  // See CoverImageUpload: open the picker explicitly instead of relying on
  // <label for> forwarding to a visually-hidden input.
  let avatarInput = $state<HTMLInputElement | null>(null);

  function openAvatarPicker() {
    avatarInput?.click();
  }
  let selectedFile = $state<File | null>(null);
  let previewUrl = $state<string | null>(null);

  // Reset transient state when modal opens.
  $effect(() => {
    if (open) {
      selectedFile = null;
      previewUrl = null;
      uploadError = undefined;
    }
  });

  // Revoke previous object URL when previewUrl changes or component unmounts.
  $effect(() => {
    const url = previewUrl;
    return () => {
      if (url) URL.revokeObjectURL(url);
    };
  });

  function computeInitials(name: string): string {
    const parts = name.trim().split(/\s+/);
    return parts.length >= 2
      ? (parts[0][0] + parts[parts.length - 1][0]).toUpperCase()
      : name.slice(0, 2).toUpperCase();
  }
  const initials = $derived(computeInitials(displayName));
  const displayAvatarUrl = $derived(previewUrl ?? avatarUrl ?? null);

  function handleFileInput(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0] ?? null;
    uploadError = undefined;
    if (!file) {
      selectedFile = null;
      previewUrl = null;
      return;
    }
    const invalid = validateImageFile(file);
    if (invalid) {
      uploadError = invalid;
      input.value = '';
      return;
    }
    selectedFile = file;
    previewUrl = URL.createObjectURL(file);
  }

  async function handleSave(e: SubmitEvent) {
    e.preventDefault();
    if (saving) return;
    saving = true;
    uploadError = undefined;

    try {
      let newAvatarUrl: string | undefined;

      if (selectedFile) {
        // FR-002: fetch a fresh short-lived auth token for each upload attempt.
        const auth = await getAvatarAuth();
        const fd = new FormData();
        fd.append('file', selectedFile);
        fd.append('fileName', selectedFile.name);
        fd.append('folder', `/avatars/${userId}/`);
        fd.append('token', auth.token);
        fd.append('expire', String(auth.expire));
        fd.append('signature', auth.signature);
        fd.append('publicKey', auth.public_key);

        const resp = await fetch('https://upload.imagekit.io/api/v1/files/upload', {
          method: 'POST',
          body: fd,
        });
        if (!resp.ok) {
          uploadError = 'Image upload failed. Please try again.';
          return;
        }
        const uploaded = (await resp.json()) as { url: string };
        newAvatarUrl = uploaded.url;
      }

      const form = e.target as HTMLFormElement;
      const display_name = (
        form.elements.namedItem('display_name') as HTMLInputElement
      ).value.trim();

      const patch: { display_name?: string; avatar_url?: string } = {};
      if (display_name) patch.display_name = display_name;
      if (newAvatarUrl) patch.avatar_url = newAvatarUrl;

      if (Object.keys(patch).length > 0) {
        await patchMe(patch);
      }

      await invalidateAll();
      notify.success('Your profile has been updated.');
      onSuccess?.();
      onClose?.();
    } catch (err) {
      if (err instanceof ApiError && err.status === 503) {
        uploadError = 'Avatar upload is not available on this server.';
      } else if (err instanceof ApiError && err.status === 400) {
        uploadError = 'Invalid avatar URL returned by upload service.';
      } else {
        uploadError = 'Something went wrong. Please try again.';
      }
    } finally {
      saving = false;
    }
  }
</script>

<Modal {open} {onClose}>
  <form onsubmit={handleSave}>
    <h2 class="mb-6 text-center font-heading text-2xl font-bold text-brand-text">
      Personal information
    </h2>

    <!-- Avatar row -->
    <div class="mb-6 flex items-center gap-5">
      <button type="button" onclick={openAvatarPicker} class="cursor-pointer shrink-0">
        {#if displayAvatarUrl}
          <img
            src={ikAvatar(displayAvatarUrl, 64)}
            alt="Avatar preview"
            class="h-16 w-16 rounded-full object-cover"
          />
        {:else}
          <div
            class="flex h-16 w-16 items-center justify-center rounded-full bg-brand-light-blue
                   font-heading text-xl font-bold text-brand-check-blue"
          >
            {initials}
          </div>
        {/if}
      </button>
      <div class="text-xs text-brand-muted">
        <p class="mb-1">Maximum file size allowed is 5 MB. Allowed types: {ACCEPTED_IMAGE_LABEL}.</p>
        <button
          type="button"
          onclick={openAvatarPicker}
          class="cursor-pointer font-semibold text-brand-blue"
        >
          {selectedFile ? selectedFile.name : 'Upload photo'}
        </button>
        <input
          bind:this={avatarInput}
          id="avatar-file-input"
          type="file"
          accept={IMAGE_ACCEPT_ATTR}
          onchange={handleFileInput}
          class="sr-only"
          tabindex="-1"
          aria-hidden="true"
        />
      </div>
    </div>

    <div class="space-y-4">
      <!-- Email (read-only) -->
      <div class="space-y-1">
        <label
          for="profile-email"
          class="block cursor-default select-none text-xs font-semibold leading-snug text-brand-text"
        >
          Your email
        </label>
        <input
          id="profile-email"
          type="email"
          value={email}
          readonly
          class="block h-12 w-full cursor-not-allowed rounded-btn border-2 border-brand-border bg-brand-light-blue px-4 py-[5px] font-body text-base text-brand-muted"
        />
        {#if emailVerified}
          <p class="mt-0.5 flex items-center gap-1 text-xs leading-none text-green-700">
            <svg class="h-3 w-3" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor">
              <path fill-rule="evenodd" d="M16.7 5.3a1 1 0 0 1 0 1.4l-7.5 7.5a1 1 0 0 1-1.4 0l-3.5-3.5a1 1 0 1 1 1.4-1.4l2.8 2.79 6.8-6.8a1 1 0 0 1 1.4 0z" clip-rule="evenodd" />
            </svg>
            Email confirmed
          </p>
        {:else}
          <p class="mt-0.5 block text-xs leading-none text-amber-700">
            Email not confirmed — use “Resend confirmation email” on your profile page
          </p>
        {/if}
      </div>

      <FormField
        label="Display name"
        type="text"
        name="display_name"
        placeholder="e.g. Andrey Kucherenko"
        value={displayName}
        required
      />

      {#if uploadError ?? error}
        <p class="text-sm text-brand-red">{uploadError ?? error}</p>
      {/if}

      <button
        type="submit"
        disabled={saving || loading}
        class="mt-2 inline-flex w-full items-center justify-center rounded-btn border-2 border-brand-blue bg-brand-blue px-5 py-[10px] font-heading text-base font-bold text-white transition-colors hover:bg-transparent hover:text-brand-blue disabled:opacity-50"
      >
        {saving ? 'Saving…' : 'Save'}
      </button>
    </div>
  </form>
</Modal>
