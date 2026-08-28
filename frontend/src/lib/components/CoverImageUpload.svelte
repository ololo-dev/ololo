<script lang="ts">
  import { ikCover } from '$lib/imagekit';
  import { browser } from '$app/environment';
  import { getAvatarAuth, ApiError } from '$lib/api';
  import { IMAGE_ACCEPT_ATTR, ACCEPTED_IMAGE_LABEL, validateImageFile } from '$lib/image-upload';

  interface Props {
    value?: string | null;
    onchange?: (url: string | null) => void;
  }

  let { value = null, onchange }: Props = $props();

  let uploading = $state(false);
  let error = $state<string | undefined>(undefined);
  let selectedFile = $state<File | null>(null);
  let previewUrl = $state<string | null>(null);
  // Opened explicitly on click rather than relying on a <label for> pointing
  // at a visually-hidden input: that forwarding is what browsers differ on.
  let fileInput = $state<HTMLInputElement | null>(null);

  function openPicker() {
    if (uploading) return;
    fileInput?.click();
  }

  const displayUrl = $derived(previewUrl ?? value ?? null);

  // Revoke object URL on change or unmount to avoid memory leaks.
  $effect(() => {
    const url = previewUrl;
    return () => {
      if (url && browser) URL.revokeObjectURL(url);
    };
  });

  function handleFileInput(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0] ?? null;
    error = undefined;

    if (!file) {
      selectedFile = null;
      previewUrl = null;
      return;
    }
    if (!accept(file)) {
      input.value = '';
      return;
    }
  }

  /** Validate, preview and upload a file from either the picker or a drop. */
  function accept(file: File): boolean {
    const invalid = validateImageFile(file);
    if (invalid) {
      error = invalid;
      return false;
    }

    selectedFile = file;
    if (browser) {
      previewUrl = URL.createObjectURL(file);
    }

    uploadFile(file);
    return true;
  }

  // Drag and drop, so a cover can be set without the OS file dialog — which
  // some browser extensions suppress.
  let dragOver = $state(false);

  function handleDragOver(e: DragEvent) {
    e.preventDefault();
    if (!uploading) dragOver = true;
  }

  function handleDragLeave() {
    dragOver = false;
  }

  function handleDrop(e: DragEvent) {
    e.preventDefault();
    dragOver = false;
    if (uploading) return;
    error = undefined;
    const file = e.dataTransfer?.files?.[0];
    if (file) accept(file);
  }

  async function uploadFile(file: File) {
    if (uploading) return;
    uploading = true;
    error = undefined;

    try {
      const auth = await getAvatarAuth();
      const fd = new FormData();
      fd.append('file', file);
      fd.append('fileName', file.name);
      fd.append('folder', '/projects/');
      fd.append('token', auth.token);
      fd.append('expire', String(auth.expire));
      fd.append('signature', auth.signature);
      fd.append('publicKey', auth.public_key);

      const resp = await fetch('https://upload.imagekit.io/api/v1/files/upload', {
        method: 'POST',
        body: fd,
      });

      if (!resp.ok) {
        error = 'Image upload failed. Please try again.';
        selectedFile = null;
        previewUrl = null;
        return;
      }

      const uploaded = (await resp.json()) as { url: string };
      onchange?.(uploaded.url);
    } catch (err) {
      if (err instanceof ApiError && err.status === 503) {
        error = 'Cover image upload is not available on this server.';
      } else if (err instanceof ApiError && err.status === 400) {
        error = 'Invalid image URL returned by upload service.';
      } else {
        error = 'Something went wrong. Please try again.';
      }
      selectedFile = null;
      previewUrl = null;
    } finally {
      uploading = false;
    }
  }

  function handleRemove() {
    selectedFile = null;
    previewUrl = null;
    error = undefined;
    onchange?.(null);
  }
</script>

<div class="space-y-3">
  {#if displayUrl}
    <div class="relative w-full overflow-hidden rounded-lg border-2 border-brand-border">
      <img
        src={ikCover(displayUrl, 1200, 320)}
        alt="Cover"
        class="h-40 w-full object-cover"
      />
      <button
        type="button"
        onclick={handleRemove}
        class="absolute right-2 top-2 rounded-btn border-2 border-brand-red bg-white px-3 py-1 font-heading text-xs font-bold text-brand-red transition-colors hover:bg-brand-red hover:text-white"
      >
        Remove
      </button>
    </div>
  {/if}

  <!-- Drop target: an alternative to the OS file dialog, which some browser
       extensions intercept. Clicking still opens the picker. -->
  <div
    role="button"
    tabindex="0"
    aria-label="Upload cover image: click to choose a file, or drop one here"
    ondragover={handleDragOver}
    ondragleave={handleDragLeave}
    ondrop={handleDrop}
    onclick={openPicker}
    onkeydown={(e) => {
      if (e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        openPicker();
      }
    }}
    class="cursor-pointer rounded-lg border-2 border-dashed p-4 text-center transition-colors
           {dragOver ? 'border-brand-blue bg-brand-blue/5' : 'border-brand-border hover:border-brand-blue/50'}
           {uploading ? 'pointer-events-none opacity-60' : ''}"
  >
    <p class="text-sm font-semibold text-brand-blue">
      {#if uploading}
        Uploading…
      {:else if dragOver}
        Drop the image to upload
      {:else}
        Drop an image here, or click to choose a file
      {/if}
    </p>
  </div>

  <div class="text-xs text-brand-muted">
    <p class="mb-1">Maximum file size allowed is 5 MB. Allowed types: {ACCEPTED_IMAGE_LABEL}.</p>
    <button
      type="button"
      onclick={openPicker}
      disabled={uploading}
      class="cursor-pointer font-semibold text-brand-blue disabled:opacity-50"
    >
      {#if uploading}
        Uploading…
      {:else if selectedFile}
        {selectedFile.name}
      {:else}
        Upload cover image
      {/if}
    </button>
    <input
      bind:this={fileInput}
      id="cover-file-input"
      type="file"
      accept={IMAGE_ACCEPT_ATTR}
      disabled={uploading}
      onchange={handleFileInput}
      class="sr-only"
      tabindex="-1"
      aria-hidden="true"
    />
  </div>

  {#if error}
    <p class="text-sm text-brand-red">{error}</p>
  {/if}
</div>
