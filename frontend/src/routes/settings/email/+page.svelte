<script lang="ts">
  import { browser } from '$app/environment';
  import { untrack } from 'svelte';
  import { notify } from '$lib/notifications.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';
  import { updateEmailTemplate, updateEmailSetting, ApiError } from '$lib/api';

  let { data } = $props();

  let emailProvider = $state(untrack(() => data.emailSettings?.['email.provider'] ?? 'ses'));
  let sesRegion = $state(untrack(() => data.emailSettings?.['email.ses_region'] ?? ''));
  let sesAccessKeyId = $state(untrack(() => data.emailSettings?.['email.access_key_id'] ?? ''));
  let sesSecretKey = $state(untrack(() => data.emailSettings?.['email.secret_access_key'] ?? '[redacted]'));
  let fromAddress = $state(untrack(() => data.emailSettings?.['email.from_address'] ?? ''));
  let cfAccountId = $state(untrack(() => data.emailSettings?.['email.cloudflare_account_id'] ?? ''));
  let cfApiToken = $state(untrack(() => data.emailSettings?.['email.cloudflare_api_token'] ?? '[redacted]'));
  let savingEmailConfig = $state(false);
  let emailConfigError = $state<string | undefined>(undefined);

  const providers = [
    { id: 'ses', label: 'AWS SES' },
    { id: 'cloudflare', label: 'Cloudflare' },
  ];

  async function saveEmailConfig() {
    savingEmailConfig = true;
    emailConfigError = undefined;
    try {
      const updates: Array<{ key: string; value: string }> = [
        { key: 'email.provider', value: emailProvider },
        { key: 'email.from_address', value: fromAddress },
      ];
      if (emailProvider === 'ses') {
        updates.push(
          { key: 'email.ses_region', value: sesRegion },
          { key: 'email.access_key_id', value: sesAccessKeyId },
        );
        if (sesSecretKey !== '[redacted]') {
          updates.push({ key: 'email.secret_access_key', value: sesSecretKey });
        }
      } else {
        updates.push({ key: 'email.cloudflare_account_id', value: cfAccountId });
        if (cfApiToken !== '[redacted]') {
          updates.push({ key: 'email.cloudflare_api_token', value: cfApiToken });
        }
      }
      for (const { key, value } of updates) {
        await updateEmailSetting(key, value);
      }
      notify.success('Email configuration saved. Restart the server to apply it.', 'Email');
    } catch {
      emailConfigError = 'Failed to save email configuration. Please try again.';
    } finally {
      savingEmailConfig = false;
    }
  }

  let templateData = $state(
    untrack(() => Object.fromEntries(
      (data.emailTemplates ?? []).map(t => [t.type, { ...t }])
    ))
  );
  let selectedTemplateType = $state('verify');
  let currentTemplate = $derived(
    templateData[selectedTemplateType] ?? { subject: '', body_html: '', body_text: '' }
  );
  let savingTemplate = $state(false);
  let templateError = $state<string | undefined>(undefined);

  const templateTypes = [
    { id: 'verify', label: 'Email Verification' },
    { id: 'reset_password', label: 'Password Reset' },
    { id: 'magic_link', label: 'Magic Link' },
  ];

  async function saveTemplate() {
    savingTemplate = true;
    templateError = undefined;
    try {
      await updateEmailTemplate(selectedTemplateType, {
        subject: currentTemplate.subject,
        body_html: currentTemplate.body_html,
        body_text: currentTemplate.body_text,
      });
      notify.success('Template saved.', 'Email');
    } catch (err) {
      if (err instanceof ApiError) {
        templateError = err.message ?? 'Failed to save template.';
      } else {
        templateError = 'Failed to save template. Please try again.';
      }
    } finally {
      savingTemplate = false;
    }
  }

  let previewOpen = $state(false);
  let previewMode = $state<'html' | 'text'>('html');

  /**
   * What each template must contain, and what to stand in for it when
   * previewing.
   */
  const placeholders: Record<string, [key: string, sample: string][]> = {
    verify: [['{{VERIFY_URL}}', 'https://ololo.dev/auth/verify-email?token=example']],
    reset_password: [['{{RESET_URL}}', 'https://ololo.dev/reset-password?token=example']],
    magic_link: [['{{MAGIC_LINK_URL}}', 'https://ololo.dev/auth/magic-link/verify?token=example']],
  };

  /** The placeholders this type requires, for the hint under the editor. */
  const requiredKeys = $derived(
    (placeholders[selectedTemplateType] ?? []).map(([key]) => key),
  );

  function substitute(body: string, plain: boolean): string {
    return (placeholders[selectedTemplateType] ?? []).reduce(
      (acc, [key, sample]) =>
        acc.replaceAll(key, plain && key === '{{CONTENT}}' ? 'The campaign body goes here.' : sample),
      body,
    );
  }

  const previewHtml = $derived(substitute(currentTemplate.body_html, false));
  const previewText = $derived(substitute(currentTemplate.body_text, true));
</script>

<div class="mt-8 flex flex-col gap-8">

  <section>
    <div class="mb-4">
      <h2 class="font-heading text-[20px] font-semibold text-brand-text">Email Sending</h2>
      <p class="mt-0.5 text-sm text-brand-muted">Configure the provider used for sending transactional emails.</p>
    </div>
    <div class="rounded-[8px] bg-white shadow-sm">
      <div class="border-b border-brand-border px-[100px] pt-4 pb-4 max-md:px-6">
        <div class="inline-flex items-center gap-1 rounded-[8px] bg-brand-light-blue p-1">
          {#each providers as provider}
            <button
              type="button"
              onclick={() => { emailProvider = provider.id; emailConfigError = undefined; }}
              class="rounded-[6px] px-4 py-1.5 text-sm font-semibold transition-colors duration-150
                     {emailProvider === provider.id
                       ? 'bg-white text-brand-text shadow-sm'
                       : 'text-brand-muted hover:text-brand-text'}"
            >
              {provider.label}
            </button>
          {/each}
        </div>
      </div>
      <div class="px-[100px] py-6 max-md:px-6">
        <div class="grid grid-cols-1 gap-6 md:grid-cols-2">
          <div>
            <div class="mb-1 text-xs font-semibold text-brand-text">From Address</div>
            <div oninput={(e) => (fromAddress = (e.target as HTMLInputElement).value)}>
              <input
                type="email"
                value={fromAddress}
                placeholder="noreply@example.com"
                class="w-full rounded-[6px] border border-brand-border px-3 py-2 text-sm
                       text-brand-text placeholder:text-brand-muted/60
                       focus:outline-none focus:ring-2 focus:ring-brand-blue"
              />
            </div>
          </div>
          {#if emailProvider === 'ses'}
            <div>
              <div class="mb-1 text-xs font-semibold text-brand-text">AWS Region</div>
              <div oninput={(e) => (sesRegion = (e.target as HTMLInputElement).value)}>
                <input
                  type="text"
                  value={sesRegion}
                  placeholder="us-east-1"
                  class="w-full rounded-[6px] border border-brand-border px-3 py-2 text-sm
                         text-brand-text placeholder:text-brand-muted/60
                         focus:outline-none focus:ring-2 focus:ring-brand-blue"
                />
              </div>
            </div>
            <div>
              <div class="mb-1 text-xs font-semibold text-brand-text">Access Key ID</div>
              <div oninput={(e) => (sesAccessKeyId = (e.target as HTMLInputElement).value)}>
                <input
                  type="text"
                  value={sesAccessKeyId}
                  placeholder="AKIAIOSFODNN7EXAMPLE"
                  class="w-full rounded-[6px] border border-brand-border px-3 py-2 text-sm
                         text-brand-text placeholder:text-brand-muted/60
                         focus:outline-none focus:ring-2 focus:ring-brand-blue"
                />
              </div>
            </div>
            <div>
              <div class="mb-1 text-xs font-semibold text-brand-text">Secret Access Key</div>
              <div oninput={(e) => (sesSecretKey = (e.target as HTMLInputElement).value)}>
                <input
                  type="password"
                  value={sesSecretKey}
                  placeholder="[redacted]"
                  class="w-full rounded-[6px] border border-brand-border px-3 py-2 text-sm
                         text-brand-text placeholder:text-brand-muted/60
                         focus:outline-none focus:ring-2 focus:ring-brand-blue"
                />
              </div>
              <p class="mt-1 text-xs text-brand-muted">Leave unchanged to keep the existing key.</p>
            </div>
          {:else}
            <div>
              <div class="mb-1 text-xs font-semibold text-brand-text">Account ID</div>
              <div oninput={(e) => (cfAccountId = (e.target as HTMLInputElement).value)}>
                <input
                  type="text"
                  value={cfAccountId}
                  placeholder="023e105f4ecef8ad9ca31a8372d0c353"
                  class="w-full rounded-[6px] border border-brand-border px-3 py-2 text-sm
                         text-brand-text placeholder:text-brand-muted/60
                         focus:outline-none focus:ring-2 focus:ring-brand-blue"
                />
              </div>
            </div>
            <div>
              <div class="mb-1 text-xs font-semibold text-brand-text">API Token</div>
              <div oninput={(e) => (cfApiToken = (e.target as HTMLInputElement).value)}>
                <input
                  type="password"
                  value={cfApiToken}
                  placeholder="[redacted]"
                  class="w-full rounded-[6px] border border-brand-border px-3 py-2 text-sm
                         text-brand-text placeholder:text-brand-muted/60
                         focus:outline-none focus:ring-2 focus:ring-brand-blue"
                />
              </div>
              <p class="mt-1 text-xs text-brand-muted">
                Needs the “Email Sending: Edit” permission; the from-address domain must be
                onboarded for Email Sending. Leave unchanged to keep the existing token.
              </p>
            </div>
          {/if}
        </div>
        {#if emailConfigError}
          <p class="mt-4 text-sm text-red-500">{emailConfigError}</p>
        {/if}
      </div>
      <div class="flex justify-end border-t border-brand-border px-[100px] py-4 max-md:px-6">
        <button
          type="button"
          disabled={savingEmailConfig}
          onclick={saveEmailConfig}
          class="rounded-btn bg-brand-blue px-6 py-2 text-sm font-semibold text-white
                 transition-opacity hover:opacity-80 disabled:cursor-not-allowed disabled:opacity-40"
        >
          {savingEmailConfig ? 'Saving…' : 'Save Email Config'}
        </button>
      </div>
    </div>
  </section>

  <section>
    <div class="mb-4">
      <h2 class="font-heading text-[20px] font-semibold text-brand-text">Email Templates</h2>
      <p class="mt-0.5 text-sm text-brand-muted">Customize the content of transactional emails sent to users.</p>
    </div>
    <div class="rounded-[8px] bg-white shadow-sm">
      <div class="border-b border-brand-border px-[100px] pt-4 max-md:px-6">
        <div class="flex flex-wrap items-center gap-1 rounded-[8px] bg-brand-light-blue p-1">
          {#each templateTypes as ttype}
            <button
              type="button"
              onclick={() => { selectedTemplateType = ttype.id; templateError = undefined; }}
              class="rounded-[6px] px-4 py-1.5 text-sm font-semibold transition-colors duration-150
                     {selectedTemplateType === ttype.id
                       ? 'bg-white text-brand-text shadow-sm'
                       : 'text-brand-muted hover:text-brand-text'}"
            >
              {ttype.label}
            </button>
          {/each}
        </div>
      </div>

      <div class="px-[100px] py-6 max-md:px-6">
        <div class="flex flex-col gap-4">
          <div>
            <div class="mb-1 text-xs font-semibold text-brand-text">Subject</div>
            <div oninput={(e) => { templateData[selectedTemplateType] = { ...currentTemplate, subject: (e.target as HTMLInputElement).value }; }}>
              <input
                type="text"
                value={currentTemplate.subject}
                placeholder="Email subject line"
                class="w-full rounded-[6px] border border-brand-border px-3 py-2 text-sm
                       text-brand-text placeholder:text-brand-muted/60
                       focus:outline-none focus:ring-2 focus:ring-brand-blue"
              />
            </div>
          </div>
          <!-- Saving is refused without these, in both bodies. Naming them
               here means finding that out while writing rather than on save. -->
          <p class="text-xs text-brand-muted" data-testid="required-placeholders">
            Must appear in both bodies:
            {#each requiredKeys as key, i (key)}<code
                class="rounded bg-brand-light-blue px-1 py-0.5 font-mono text-[11px] text-brand-blue"
                >{key}</code
              >{#if i < requiredKeys.length - 1}, {/if}{/each}
          </p>
          <div>
            <div class="mb-1 text-xs font-semibold text-brand-text">HTML Body</div>
            <div oninput={(e) => { templateData[selectedTemplateType] = { ...currentTemplate, body_html: (e.target as HTMLTextAreaElement).value }; }}>
              <textarea
                value={currentTemplate.body_html}
                placeholder="HTML email content…"
                rows={10}
                class="w-full rounded-[6px] border border-brand-border px-3 py-2 font-mono text-xs
                       text-brand-text placeholder:text-brand-muted/60
                       focus:outline-none focus:ring-2 focus:ring-brand-blue"
              ></textarea>
            </div>
          </div>
          <div>
            <div class="mb-1 text-xs font-semibold text-brand-text">Plain-text Body</div>
            <div oninput={(e) => { templateData[selectedTemplateType] = { ...currentTemplate, body_text: (e.target as HTMLTextAreaElement).value }; }}>
              <textarea
                value={currentTemplate.body_text}
                placeholder="Plain-text email content…"
                rows={6}
                class="w-full rounded-[6px] border border-brand-border px-3 py-2 font-mono text-xs
                       text-brand-text placeholder:text-brand-muted/60
                       focus:outline-none focus:ring-2 focus:ring-brand-blue"
              ></textarea>
            </div>
          </div>
        </div>
        {#if templateError}
          <p class="mt-4 text-sm text-red-500">{templateError}</p>
        {/if}
      </div>
      <div class="flex items-center justify-end gap-3 border-t border-brand-border px-[100px] py-4 max-md:px-6">
        <button
          type="button"
          onclick={() => { previewOpen = true; previewMode = 'html'; }}
          class="rounded-btn border border-brand-border px-6 py-2 text-sm font-semibold text-brand-text
                 transition-colors hover:border-brand-blue hover:text-brand-blue"
        >
          Preview
        </button>
        <button
          type="button"
          disabled={savingTemplate}
          onclick={saveTemplate}
          class="rounded-btn bg-brand-blue px-6 py-2 text-sm font-semibold text-white
                 transition-opacity hover:opacity-80 disabled:cursor-not-allowed disabled:opacity-40"
        >
          {savingTemplate ? 'Saving…' : 'Save Template'}
        </button>
      </div>
    </div>
  </section>

</div>

<Modal open={previewOpen} onClose={() => (previewOpen = false)} maxWidth="md">
  <h2 class="mb-4 font-heading text-xl font-bold text-brand-text">
    Email Preview — {templateTypes.find(t => t.id === selectedTemplateType)?.label}
  </h2>

  <div class="mb-4 inline-flex items-center gap-1 rounded-[8px] bg-brand-light-blue p-1">
    <button
      type="button"
      onclick={() => (previewMode = 'html')}
      class="rounded-[6px] px-4 py-1.5 text-sm font-semibold transition-colors duration-150
             {previewMode === 'html' ? 'bg-white text-brand-text shadow-sm' : 'text-brand-muted hover:text-brand-text'}"
    >
      HTML
    </button>
    <button
      type="button"
      onclick={() => (previewMode = 'text')}
      class="rounded-[6px] px-4 py-1.5 text-sm font-semibold transition-colors duration-150
             {previewMode === 'text' ? 'bg-white text-brand-text shadow-sm' : 'text-brand-muted hover:text-brand-text'}"
    >
      Plain text
    </button>
  </div>

  {#if previewMode === 'html'}
    {#if browser}
      <iframe
        srcdoc={previewHtml}
        title="Email HTML preview"
        class="h-[480px] w-full rounded-[6px] border border-brand-border bg-white"
        sandbox="allow-same-origin"
      ></iframe>
    {/if}
  {:else}
    <pre class="h-[480px] w-full overflow-auto rounded-[6px] border border-brand-border bg-brand-light-blue p-4
               font-mono text-xs text-brand-text whitespace-pre-wrap">{previewText}</pre>
  {/if}

  <div class="mt-4 flex justify-end">
    <button
      type="button"
      onclick={() => (previewOpen = false)}
      class="rounded-btn bg-brand-blue px-6 py-2 text-sm font-semibold text-white
             transition-opacity hover:opacity-80"
    >
      Close
    </button>
  </div>
</Modal>