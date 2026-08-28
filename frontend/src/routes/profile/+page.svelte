<script lang="ts">
  import { ikAvatar } from '$lib/imagekit';
  import { enhance } from '$app/forms';
  import { invalidateAll } from '$app/navigation';
  import { untrack } from 'svelte';
  import { browser } from '$app/environment';
  import { notify } from '$lib/notifications.svelte';
  import { WsProjectClient } from '$lib/ws-project.svelte';
  import { resendVerification, ApiError, type Session } from '$lib/api';
  import ChangePasswordModal from '$lib/components/ChangePasswordModal.svelte';
  import ProfileEditModal from '$lib/components/ProfileEditModal.svelte';
  import { formatCountdown, formatDateUTC as formatDate } from '$lib/format';
  import { statusClass, statusLabels, sessionLinkLabel } from '$lib/session-status';

  let { data } = $props();

  // ---------- Tab state ----------
  type Tab = 'sessions' | 'keys' | 'account';
  let activeTab = $state<Tab>('account');

  const tabs: { id: Tab; label: string }[] = [
    { id: 'account',  label: 'Account' },
    { id: 'keys',     label: 'Keys' },
    { id: 'sessions', label: 'Sessions' },
  ];

  // ---------- Email verification ----------
  let resendingVerification = $state(false);
  let verificationSent = $state(false);

  async function handleResendVerification() {
    resendingVerification = true;
    try {
      await resendVerification();
      verificationSent = true;
      notify.success('Confirmation email sent — check your inbox.');
    } catch (err) {
      const rateLimited = err instanceof ApiError && err.status === 429;
      notify.error(
        rateLimited
          ? 'Too many requests. Please wait a moment and try again.'
          : 'Could not send the confirmation email. Please try again later.'
      );
    } finally {
      resendingVerification = false;
    }
  }

  // Live sessions state — starts from SSR data, newest first, updated by WS
  let liveSessions = $state<Session[]>(untrack(() =>
    data.sessions
      .map(s => ({ ...s }))
      .sort((a, b) => new Date(b.created_at).getTime() - new Date(a.created_at).getTime())
  ));

  // WS clients keyed by project_id — $state so template reads sessionCountdowns reactively
  let projectClients = $state<Record<string, WsProjectClient>>({});

  $effect(() => {
    if (!browser) return;
    const pids = [...new Set(data.sessions.map(s => s.project_id))].filter(Boolean);
    const newClients: Record<string, WsProjectClient> = {};
    for (const pid of pids) {
      const c = new WsProjectClient(pid, (frame) => {
        liveSessions = liveSessions.map(s =>
          s.id === frame.session_id ? { ...s, status: frame.status } : s
        );
      });
      c.connect();
      newClients[pid] = c;
    }
    projectClients = newClients;
    return () => {
      for (const c of Object.values(newClients)) c.disconnect();
      projectClients = {};
    };
  });

  // ---------- Keys tab ----------
  let revokingId = $state<string | null>(null);

  // ---------- Account tab ----------
  let username = $state(untrack(() => data.profile.username ?? ''));
  $effect(() => { username = data.profile.username ?? ''; });

  let savingUsername = $state(false);
  let changePasswordOpen = $state(false);
  let profileEditOpen = $state(false);

  function getInitials(name: string): string {
    const parts = name.trim().split(/\s+/);
    if (parts.length >= 2) return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
    return name.slice(0, 2).toUpperCase();
  }
</script>

<svelte:head>
  <title>Profile — ololo.dev</title>
</svelte:head>

<div class="-mx-6 -mt-8 min-h-screen bg-brand-light-blue">
  <div class="mx-auto w-full max-w-[1206px] px-[18px] py-10 md:py-[88px]">

    <!-- Page header -->
    <h1 class="font-heading text-[34px] font-bold leading-[1.18] text-brand-text">
      Profile
    </h1>

    <!-- Tab bar -->
    <div class="-mx-[18px] mt-8 overflow-x-auto px-[18px] md:mx-0 md:px-0">
    <div class="inline-flex items-center gap-1 whitespace-nowrap rounded-[10px] bg-white p-1 shadow-sm">
      {#each tabs as tab}
        <button
          type="button"
          onclick={() => (activeTab = tab.id)}
          class="rounded-[7px] px-5 py-1.5 text-sm font-semibold transition-colors duration-150
                 {activeTab === tab.id
                   ? 'bg-brand-blue text-white shadow-sm'
                   : 'text-brand-muted hover:text-brand-text'}"
        >
          {tab.label}
          {#if tab.id === 'sessions'}
            <span
              class="ml-1.5 rounded-full px-1.5 py-px text-[10px] font-bold
                     {activeTab === 'sessions' ? 'bg-white/20 text-white' : 'bg-brand-blue/10 text-brand-blue'}"
            >
              {liveSessions.length}
            </span>
          {/if}
        </button>
      {/each}
    </div>
    </div>

    <!-- ================================================================
         SESSIONS TAB
    ================================================================ -->
    {#if activeTab === 'sessions'}
      <div class="mt-8">
        <div class="mb-4">
          <h2 class="font-heading text-[20px] font-semibold text-brand-text">Sessions</h2>
          <p class="mt-0.5 text-sm text-brand-muted">
            {liveSessions.length}
            {liveSessions.length === 1 ? 'session' : 'sessions'}
          </p>
        </div>

        <div class="overflow-x-auto rounded-[8px] bg-white shadow-sm">
          {#if liveSessions.length === 0}
            <div class="flex flex-col items-center justify-center py-16 text-brand-muted">
              <svg width="40" height="40" viewBox="0 0 24 24" fill="none" class="mb-3 opacity-40" aria-hidden="true">
                <rect x="3" y="3" width="18" height="18" rx="2" stroke="currentColor" stroke-width="2" />
                <path d="M3 9h18" stroke="currentColor" stroke-width="2" />
              </svg>
              <p class="text-sm">No sessions yet.</p>
            </div>
          {:else}
              <table class="w-full">
              <thead>
                <tr class="border-b border-brand-border bg-brand-light-blue/40">
                  <th class="px-6 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Name</th>
                  <th class="px-6 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Status</th>
                  <th class="px-6 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Countdown</th>
                  <th class="px-6 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Created</th>
                  <th class="px-6 py-3 text-right text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Actions</th>
                </tr>
              </thead>
              <tbody>
                {#each liveSessions as session (session.id)}
                  {@const cd = projectClients[session.project_id]?.sessionCountdowns[session.id] ?? null}
                  <tr class="border-b border-brand-border/60 last:border-0 transition-colors hover:bg-brand-light-blue/30">
                    <td class="px-6 py-4 text-sm font-medium text-brand-text">{session.name}</td>
                    <td class="px-6 py-4">
                      <span class="inline-flex items-center rounded-full px-2.5 py-0.5 text-[11px] font-semibold {statusClass(session.status)}">
                        {statusLabels[session.status] ?? session.status}
                      </span>
                    </td>
                    <td class="px-6 py-4 text-sm text-brand-muted">
                      {#if cd}
                        <span class="font-mono text-[11px] {cd.type === 'running' ? 'text-green-600' : 'text-amber-600'}">
                          {cd.type === 'lobby' ? 'Starts in ' : ''}{formatCountdown(cd.secs)}
                        </span>
                      {:else}
                        —
                      {/if}
                    </td>
                    <td class="px-6 py-4 text-sm text-brand-muted">{formatDate(session.created_at)}</td>
                    <td class="px-6 py-4 text-right">
                      {#if session.join_code}
                        <a
                          href="/s/{session.join_code}"
                          class="whitespace-nowrap rounded px-3 py-1 text-xs font-semibold text-brand-blue transition-colors hover:bg-brand-blue/10"
                        >
                          {sessionLinkLabel(session.status)} →
                        </a>
                      {:else}
                        <span class="rounded px-3 py-1 text-xs font-semibold text-brand-muted">Unavailable</span>
                      {/if}
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {/if}
        </div>
      </div>
    {/if}

    <!-- ================================================================
         KEYS TAB
    ================================================================ -->
    {#if activeTab === 'keys'}
      <div class="mt-8">
        <div class="mb-4">
          <h2 class="font-heading text-[20px] font-semibold text-brand-text">API Keys</h2>
          <p class="mt-0.5 text-sm text-brand-muted">
            Personal access tokens used by the observer CLI to authenticate on your behalf.
          </p>
        </div>

        <div class="overflow-x-auto rounded-[8px] bg-white shadow-sm">
          {#if data.pats.length === 0}
            <div class="flex flex-col items-center justify-center py-16 text-brand-muted">
              <svg width="40" height="40" viewBox="0 0 24 24" fill="none" class="mb-3 opacity-40" aria-hidden="true">
                <path d="M21 2l-2 2m-7.61 7.61a5.5 5.5 0 1 1-7.778 7.778 5.5 5.5 0 0 1 7.777-7.777zm0 0L15.5 7.5m0 0l3 3L22 7l-3-3m-3.5 3.5L19 4" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
              </svg>
              <p class="text-sm">No API keys yet. Register an agent to generate one.</p>
            </div>
          {:else}
            <table class="w-full">
              <thead>
                <tr class="border-b border-brand-border bg-brand-light-blue/40">
                  <th class="px-6 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Key</th>
                  <th class="px-6 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Created</th>
                  <th class="px-6 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Expires</th>
                  <th class="px-6 py-3 text-right text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Actions</th>
                </tr>
              </thead>
              <tbody>
                {#each data.pats as pat (pat.id)}
                  <tr class="border-b border-brand-border/60 last:border-0 transition-colors hover:bg-brand-light-blue/30">
                    <td class="px-6 py-4">
                      <code class="rounded bg-brand-light-blue px-2 py-0.5 font-mono text-xs text-brand-text">
                        ololo_{pat.fingerprint}...
                      </code>
                    </td>
                    <td class="px-6 py-4 text-sm text-brand-muted">{formatDate(pat.created_at)}</td>
                    <td class="px-6 py-4 text-sm text-brand-muted">
                      {pat.expires_at ? formatDate(pat.expires_at) : 'Never'}
                    </td>
                    <td class="px-6 py-4 text-right">
                      <form
                        method="POST"
                        action="?/revokePat"
                        use:enhance={() => {
                          revokingId = pat.id;
                          return async ({ result, update }) => {
                            await update();
                            revokingId = null;
                            if (result.type === 'success' && (result.data as { success?: boolean })?.success) {
                              notify.success('API key revoked.', 'Keys');
                            } else {
                              notify.error('Failed to revoke API key.', 'Keys');
                            }
                          };
                        }}
                      >
                        <input type="hidden" name="id" value={pat.id} />
                        <button
                          type="submit"
                          disabled={revokingId === pat.id}
                          class="rounded px-3 py-1 text-xs font-semibold text-red-500 transition-colors hover:bg-red-50 disabled:opacity-40"
                        >
                          {revokingId === pat.id ? 'Revoking…' : 'Revoke'}
                        </button>
                      </form>
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {/if}
        </div>
      </div>
    {/if}

    <!-- ================================================================
         ACCOUNT TAB
    ================================================================ -->
    {#if activeTab === 'account'}
      <div class="mt-8 flex flex-col gap-8">

        <!-- Profile card -->
        <section>
          <div class="mb-4">
            <h2 class="font-heading text-[20px] font-semibold text-brand-text">Profile</h2>
            <p class="mt-0.5 text-sm text-brand-muted">Your public display information.</p>
          </div>
          <div class="rounded-[8px] bg-white shadow-sm">
            <div class="flex flex-wrap items-center gap-6 px-5 py-6 sm:px-8">
              {#if data.profile.avatar_url}
                <img
                  src={ikAvatar(data.profile.avatar_url, 64)}
                  alt={data.profile.display_name}
                  class="h-16 w-16 rounded-full object-cover"
                />
              {:else}
                <div class="flex h-16 w-16 shrink-0 items-center justify-center rounded-full bg-brand-blue/10 text-lg font-bold text-brand-blue">
                  {getInitials(data.profile.display_name)}
                </div>
              {/if}
              <div class="flex-1">
                <p class="text-lg font-semibold text-brand-text">{data.profile.display_name}</p>
                <p class="flex flex-wrap items-center gap-2 text-sm text-brand-muted">
                  {data.profile.email}
                  {#if data.profile.email_verified}
                    <span
                      class="inline-flex items-center gap-1 rounded-full bg-green-100 px-2.5 py-0.5 text-[11px] font-semibold text-green-700"
                      title="Your email address is confirmed"
                    >
                      <svg class="h-3 w-3" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor">
                        <path fill-rule="evenodd" d="M16.7 5.3a1 1 0 0 1 0 1.4l-7.5 7.5a1 1 0 0 1-1.4 0l-3.5-3.5a1 1 0 1 1 1.4-1.4l2.8 2.79 6.8-6.8a1 1 0 0 1 1.4 0z" clip-rule="evenodd" />
                      </svg>
                      Confirmed
                    </span>
                  {:else}
                    <span
                      class="inline-flex items-center gap-1 rounded-full bg-amber-100 px-2.5 py-0.5 text-[11px] font-semibold text-amber-700"
                      title="Your email address has not been confirmed yet"
                    >
                      Not confirmed
                    </span>
                  {/if}
                </p>
                {#if !data.profile.email_verified}
                  <p class="mt-1 text-xs text-brand-muted">
                    Confirm your email to secure your account and receive sign-in links.
                    {#if verificationSent}
                      <span class="font-semibold text-green-700">Email sent — check your inbox.</span>
                    {:else}
                      <button
                        type="button"
                        onclick={handleResendVerification}
                        disabled={resendingVerification}
                        class="font-semibold text-brand-blue transition-opacity hover:opacity-60 disabled:opacity-50 bg-transparent"
                      >
                        {resendingVerification ? 'Sending…' : 'Resend confirmation email'}
                      </button>
                    {/if}
                  </p>
                {/if}
                {#if data.profile.is_admin}
                  <span class="mt-1 inline-flex items-center rounded-full bg-brand-blue/10 px-2.5 py-0.5 text-[11px] font-semibold text-brand-blue">
                    Admin
                  </span>
                {/if}
              </div>
              <button
                type="button"
                onclick={() => (profileEditOpen = true)}
                class="rounded-btn border-2 border-brand-blue px-5 py-2 text-sm font-semibold text-brand-blue
                       transition-colors hover:bg-brand-blue hover:text-white"
              >
                Edit
              </button>
            </div>
          </div>
        </section>

        <!-- Judge-run quota (only when the instance enforces limits) -->
        {#if data.profile.plans_enabled}
          {@const used = data.profile.judge_runs_used}
          {@const limit = data.profile.judge_run_limit}
          {@const pct = limit > 0 ? Math.min(100, Math.round((used / limit) * 100)) : 100}
          <section>
            <div class="mb-4">
              <h2 class="font-heading text-[20px] font-semibold text-brand-text">Judge reviews</h2>
              <p class="mt-0.5 text-sm text-brand-muted">
                AI judge reviews of your play, counted per calendar month.
              </p>
            </div>
            <div class="rounded-[8px] bg-white shadow-sm">
              <div class="px-5 py-6 sm:px-8">
                <div class="mb-3 flex flex-wrap items-center justify-end gap-2">
                  <span class="text-sm {used >= limit && data.profile.judge_run_credits <= 0 ? 'font-semibold text-brand-red' : 'text-brand-muted'}">
                    <span class="font-semibold text-brand-text">{used}</span>
                    / {limit} judge runs this month{#if data.profile.judge_run_credits > 0}
                      <span class="text-brand-blue"> +{data.profile.judge_run_credits.toLocaleString('en-US')} extra</span>{/if}
                  </span>
                </div>
                <div class="h-2 w-full overflow-hidden rounded-full bg-gray-100">
                  <div
                    class="h-full rounded-full transition-[width] duration-300
                           {used >= limit ? 'bg-brand-red' : pct >= 80 ? 'bg-amber-400' : 'bg-brand-blue'}"
                    style:width="{pct}%"
                  ></div>
                </div>
                <p class="mt-2 text-xs text-brand-muted">
                  {#if used >= limit}
                    Limit reached — judge reviews resume on the 1st of next month (UTC).
                  {:else}
                    Resets on the 1st of each month (UTC).
                  {/if}
                </p>
              </div>
            </div>
          </section>
        {/if}

        <!-- Username -->
        <section>
          <div class="mb-4">
            <h2 class="font-heading text-[20px] font-semibold text-brand-text">Username</h2>
            <p class="mt-0.5 text-sm text-brand-muted">
              Your public handle at <code class="text-brand-text">/u/{username || '…'}</code>.
              Changing it will break existing links.
            </p>
          </div>
          <div class="rounded-[8px] bg-white shadow-sm">
            <form
              method="POST"
              action="?/updateUsername"
              use:enhance={() => {
                savingUsername = true;
                return async ({ result, update }) => {
                  await update({ reset: false });
                  savingUsername = false;
                  if (result.type === 'success' && (result.data as { success?: boolean })?.success) {
                    await invalidateAll();
                    notify.success('Username updated.', 'Account');
                  } else if (result.type === 'success') {
                    const code = (result.data as { error?: string })?.error ?? 'error';
                    if (code === 'username_taken') {
                      notify.error('That username is already taken.', 'Account');
                    } else if (code === 'invalid_username') {
                      notify.error('Invalid username format.', 'Account');
                    } else {
                      notify.error('Failed to update username. Please try again.', 'Account');
                    }
                  }
                };
              }}
            >
              <div class="px-5 py-6 sm:px-8">
                <div class="max-w-sm">
                  <label for="username" class="mb-1 block text-xs font-semibold text-brand-text">
                    Username
                  </label>
                  <input
                    id="username"
                    name="username"
                    type="text"
                    bind:value={username}
                    required
                    minlength={4}
                    maxlength={30}
                    pattern="[a-z][a-z0-9_\-]*[a-z0-9]"
                    class="w-full rounded-[6px] border border-brand-border px-3 py-2 text-sm text-brand-text
                           placeholder:text-brand-muted/60 focus:outline-none focus:ring-2 focus:ring-brand-blue"
                    placeholder="e.g. coolazurerose"
                  />
                  <p class="mt-1 text-[11px] text-brand-muted">
                    4–30 characters. Lowercase letters, digits, hyphens, and underscores.
                    Must start with a letter and end with a letter or digit.
                  </p>
                </div>
              </div>
              <div class="flex justify-end border-t border-brand-border px-5 py-4 sm:px-8">
                <button
                  type="submit"
                  disabled={savingUsername || !username.trim()}
                  class="rounded-btn bg-brand-blue px-6 py-2 text-sm font-semibold text-white
                         transition-opacity hover:opacity-80 disabled:cursor-not-allowed disabled:opacity-40"
                >
                  {savingUsername ? 'Saving…' : 'Save Username'}
                </button>
              </div>
            </form>
          </div>
        </section>

        <!-- Password -->
        <section>
          <div class="mb-4">
            <h2 class="font-heading text-[20px] font-semibold text-brand-text">Password</h2>
            <p class="mt-0.5 text-sm text-brand-muted">Update your account password.</p>
          </div>
          <div class="rounded-[8px] bg-white shadow-sm">
            <div class="flex flex-wrap items-center justify-between gap-4 px-5 py-6 sm:px-8">
              <p class="text-sm text-brand-muted">Use a strong, unique password for your account.</p>
              <button
                type="button"
                onclick={() => (changePasswordOpen = true)}
                class="rounded-btn border-2 border-brand-blue px-5 py-2 text-sm font-semibold text-brand-blue
                       transition-colors hover:bg-brand-blue hover:text-white"
              >
                Change Password
              </button>
            </div>
          </div>
        </section>

      </div>
    {/if}

  </div>
</div>

<ChangePasswordModal
  open={changePasswordOpen}
  action="?/changePassword"
  onClose={() => (changePasswordOpen = false)}
/>

<ProfileEditModal
  open={profileEditOpen}
  displayName={data.profile.display_name}
  email={data.profile.email}
  emailVerified={data.profile.email_verified}
  userId={data.profile.id}
  avatarUrl={data.profile.avatar_url}
  onClose={() => (profileEditOpen = false)}
/>
