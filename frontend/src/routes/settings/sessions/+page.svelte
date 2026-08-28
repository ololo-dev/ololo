<script lang="ts">
  // The instance-wide session registry.
  //
  // Every other view of a session is scoped to someone: a project's sessions,
  // a player's history, your own list. This is the only one that can answer
  // "what is running right now, and who started it" across the whole
  // instance, which is what an admin chasing a stuck session actually needs.

  import { goto, invalidateAll } from '$app/navigation';
  import { page } from '$app/stores';
  import { untrack } from 'svelte';
  import { notify } from '$lib/notifications.svelte';
  import { formatDateTimeUTC, formatDateUTC, formatTimeUTC } from '$lib/format';
  import { statusClass, statusLabels } from '$lib/session-status';
  import { patchSession, deleteSession, ApiError, type AdminSession } from '$lib/api';

  let { data } = $props();

  const STATUS_OPTIONS = [
    { value: '', label: 'Any status' },
    { value: 'lobby', label: 'In Lobby' },
    { value: 'running', label: 'Running' },
    { value: 'paused', label: 'Paused' },
    { value: 'finished', label: 'Finished' },
    { value: 'cancelled', label: 'Cancelled' },
  ];

  /** Search box is local until submitted; the rest apply on change. */
  let search = $state(untrack(() => data.filters.q));
  $effect(() => { search = data.filters.q; });

  let busyId = $state<string | null>(null);

  const perPage = $derived(data.sessions.per_page || 25);
  const pageNo = $derived(data.sessions.page || 1);
  const lastPage = $derived(Math.max(1, Math.ceil(data.sessions.total / perPage)));
  const firstShown = $derived(
    data.sessions.total === 0 ? 0 : (pageNo - 1) * perPage + 1,
  );
  const lastShown = $derived(Math.min(pageNo * perPage, data.sessions.total));

  /**
   * Filters live in the URL, so navigating is how they are applied: a reload
   * after a cancel then lands on the same filtered view rather than resetting
   * to every session on the instance.
   */
  function applyFilters(changes: Record<string, string>, resetPage = true) {
    const params = new URLSearchParams($page.url.searchParams);
    for (const [key, value] of Object.entries(changes)) {
      if (value) params.set(key, value);
      else params.delete(key);
    }
    if (resetPage) params.delete('page');
    const qs = params.toString();
    goto(`/settings/sessions${qs ? `?${qs}` : ''}`, { keepFocus: true, noScroll: true });
  }

  function gotoPage(n: number) {
    applyFilters({ page: n <= 1 ? '' : String(n) }, false);
  }

  /** Only a session that has not ended can still be cancelled. */
  function isLive(s: AdminSession): boolean {
    return s.status === 'lobby' || s.status === 'running' || s.status === 'paused';
  }

  async function cancelSession(s: AdminSession) {
    if (!confirm(`Cancel session ${s.join_code} ("${s.name}")? Players will be disconnected.`)) {
      return;
    }
    busyId = s.id;
    try {
      await patchSession(s.id, { status: 'cancelled' });
      notify.success(`${s.join_code} cancelled.`, 'Sessions');
      await invalidateAll();
    } catch (err) {
      const conflict = err instanceof ApiError && err.status === 409;
      notify.error(
        conflict ? `${s.join_code} has already ended.` : 'Could not cancel the session.',
        'Sessions',
      );
    } finally {
      busyId = null;
    }
  }

  async function removeSession(s: AdminSession) {
    if (
      !confirm(
        `Delete session ${s.join_code} ("${s.name}")? Its players and results go with it. This cannot be undone.`,
      )
    ) {
      return;
    }
    busyId = s.id;
    try {
      await deleteSession(s.id);
      notify.success(`${s.join_code} deleted.`, 'Sessions');
      await invalidateAll();
    } catch {
      notify.error('Could not delete the session.', 'Sessions');
    } finally {
      busyId = null;
    }
  }
</script>

<div class="flex flex-col gap-6">
  <div>
    <h2 class="font-heading text-[20px] font-semibold text-brand-text">Sessions</h2>
    <p class="mt-0.5 text-sm text-brand-muted">
      Every session on this instance, newest first — including ones you neither own nor joined.
    </p>
  </div>

  <!-- Filters -->
  <div class="flex flex-col gap-3 sm:flex-row sm:items-center">
    <form
      class="flex min-w-0 flex-1 gap-2"
      onsubmit={(e) => {
        e.preventDefault();
        applyFilters({ q: search.trim() });
      }}
    >
      <input
        type="search"
        value={search}
        oninput={(e) => (search = e.currentTarget.value)}
        placeholder="Join code or name…"
        aria-label="Search sessions"
        class="min-w-0 flex-1 rounded-[6px] border border-brand-border px-3 py-2 text-sm text-brand-text
               focus:outline-none focus:ring-2 focus:ring-brand-blue"
      />
      <button
        type="submit"
        class="shrink-0 rounded-btn bg-brand-blue px-4 py-2 text-sm font-semibold text-white
               transition-opacity hover:opacity-80"
      >
        Search
      </button>
    </form>

    <select
      value={data.filters.status}
      onchange={(e) => applyFilters({ status: e.currentTarget.value })}
      aria-label="Filter by status"
      class="rounded-[6px] border border-brand-border px-3 py-2 text-sm text-brand-text
             focus:outline-none focus:ring-2 focus:ring-brand-blue"
    >
      {#each STATUS_OPTIONS as opt (opt.value)}
        <option value={opt.value}>{opt.label}</option>
      {/each}
    </select>

    <select
      value={data.filters.projectId}
      onchange={(e) => applyFilters({ project_id: e.currentTarget.value })}
      aria-label="Filter by project"
      class="max-w-[220px] rounded-[6px] border border-brand-border px-3 py-2 text-sm text-brand-text
             focus:outline-none focus:ring-2 focus:ring-brand-blue"
    >
      <option value="">Any project</option>
      {#each data.projects as p (p.id)}
        <option value={p.id}>{p.name}</option>
      {/each}
    </select>
  </div>

  <div class="rounded-[8px] bg-white shadow-sm" data-testid="admin-sessions">
    {#if data.sessions.sessions.length === 0}
      <div class="flex flex-col items-center justify-center py-16 text-brand-muted">
        <p class="text-sm">
          {#if data.filters.q || data.filters.status || data.filters.projectId}
            No sessions match these filters.
          {:else}
            No sessions have been created yet.
          {/if}
        </p>
      </div>
    {:else}
      <!-- Cards below `xl`, the same way the AI providers list works: the
           settings column is ~940px wide, which seven columns do not fit. -->
      <ul class="divide-y divide-brand-border/60 xl:hidden">
        {#each data.sessions.sessions as s (s.id)}
          <li class="flex flex-col gap-2.5 p-4" data-testid="session-card-{s.join_code}">
            <div class="flex items-start justify-between gap-3">
              <span class="flex min-w-0 flex-col leading-tight">
                <a
                  href="/s/{s.join_code}"
                  class="font-mono text-sm font-bold text-brand-blue hover:underline"
                >{s.join_code}</a>
                <span class="truncate text-xs text-brand-muted">{s.name}</span>
              </span>
              <span
                class="shrink-0 rounded-full px-2.5 py-0.5 text-[11px] font-semibold {statusClass(s.status)}"
              >{statusLabels[s.status] ?? s.status}</span>
            </div>

            <dl class="grid grid-cols-[4.5rem_minmax(0,1fr)] items-center gap-x-3 gap-y-1.5 text-xs text-brand-muted">
              <dt class="font-semibold uppercase tracking-wider text-brand-muted/70">Project</dt>
              <dd class="min-w-0 truncate">{s.project_name ?? '—'}</dd>
              <dt class="font-semibold uppercase tracking-wider text-brand-muted/70">Owner</dt>
              <dd class="min-w-0 truncate">{s.owner_display_name ?? '—'}</dd>
              <dt class="font-semibold uppercase tracking-wider text-brand-muted/70">Players</dt>
              <dd>{s.player_count}</dd>
              <dt class="font-semibold uppercase tracking-wider text-brand-muted/70">Created</dt>
              <dd>{formatDateTimeUTC(s.created_at)}</dd>
            </dl>

            <div class="flex flex-wrap justify-end gap-2">
              {#if isLive(s)}
                <button
                  type="button"
                  onclick={() => cancelSession(s)}
                  disabled={busyId === s.id}
                  class="rounded px-3 py-1 text-xs font-semibold text-amber-700
                         transition-colors hover:bg-amber-50 disabled:opacity-40"
                >Cancel</button>
              {/if}
              <button
                type="button"
                onclick={() => removeSession(s)}
                disabled={busyId === s.id}
                class="rounded px-3 py-1 text-xs font-semibold text-red-500
                       transition-colors hover:bg-red-50 disabled:opacity-40"
              >Delete</button>
            </div>
          </li>
        {/each}
      </ul>

      <!-- table-fixed with flexible name columns: the two text columns absorb
           whatever width is left and truncate, so the table never grows wider
           than the settings column and never needs a horizontal scrollbar. -->
      <div class="hidden xl:block">
        <table class="w-full table-fixed">
          <thead>
            <tr class="border-b border-brand-border bg-brand-light-blue/40 text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
              <th class="w-[24%] whitespace-nowrap py-3 pl-6 pr-4 text-left">Session</th>
              <th class="whitespace-nowrap px-4 py-3 text-left">Project / Owner</th>
              <th class="w-[92px] whitespace-nowrap px-4 py-3 text-left">Status</th>
              <th class="w-[76px] whitespace-nowrap px-4 py-3 text-right">Players</th>
              <th class="w-[116px] whitespace-nowrap px-4 py-3 text-left">Created</th>
              <th class="w-[172px] whitespace-nowrap py-3 pl-4 pr-6 text-right">Actions</th>
            </tr>
          </thead>
          <tbody>
            {#each data.sessions.sessions as s (s.id)}
              <tr
                class="border-b border-brand-border/60 last:border-0 hover:bg-brand-light-blue/20"
                data-testid="session-row-{s.join_code}"
              >
                <td class="py-3 pl-6 pr-4">
                  <span class="flex min-w-0 flex-col leading-tight">
                    <a
                      href="/s/{s.join_code}"
                      class="font-mono text-sm font-bold text-brand-blue hover:underline"
                    >{s.join_code}</a>
                    <span class="truncate text-xs text-brand-muted" title={s.name}>
                      {s.name}
                    </span>
                  </span>
                </td>
                <!-- Project and owner share a column: seven columns wanted
                     1080px and this one gets 940, and these two are the pair
                     that reads naturally stacked ("this kata, run by them"). -->
                <td class="px-4 py-3 text-sm text-brand-muted">
                  <span class="flex min-w-0 flex-col leading-tight">
                    {#if s.project_name}
                      <a
                        href="/projects/{s.project_slug ?? s.project_id}"
                        class="truncate hover:text-brand-blue hover:underline"
                        title={s.project_name}
                      >{s.project_name}</a>
                    {:else}
                      <span class="text-brand-muted/60">—</span>
                    {/if}
                    {#if s.owner_username}
                      <a
                        href="/u/{s.owner_username}"
                        class="truncate text-xs hover:text-brand-blue hover:underline"
                      >{s.owner_display_name ?? s.owner_username}</a>
                    {:else if s.owner_display_name}
                      <span class="truncate text-xs">{s.owner_display_name}</span>
                    {:else}
                      <span class="text-xs text-brand-muted/60">—</span>
                    {/if}
                  </span>
                </td>
                <td class="whitespace-nowrap px-4 py-3">
                  <span
                    class="rounded-full px-2.5 py-0.5 text-[11px] font-semibold {statusClass(s.status)}"
                    title={s.cancel_reason
                      ? `Cancelled: ${s.cancel_reason}${s.cancelled_by ? ` by ${s.cancelled_by}` : ''}`
                      : undefined}
                  >{statusLabels[s.status] ?? s.status}</span>
                </td>
                <td class="whitespace-nowrap px-4 py-3 text-right text-sm tabular-nums text-brand-muted">
                  {s.player_count}
                </td>
                <td class="whitespace-nowrap px-4 py-3 text-sm text-brand-muted">
                  <span class="flex flex-col leading-tight" title={formatDateTimeUTC(s.created_at)}>
                    <span>{formatDateUTC(s.created_at)}</span>
                    <span class="text-xs text-brand-muted/80">{formatTimeUTC(s.created_at)}</span>
                  </span>
                </td>
                <td class="whitespace-nowrap py-3 pl-4 pr-6 text-right">
                  <div class="flex justify-end gap-1">
                    {#if isLive(s)}
                      <button
                        type="button"
                        onclick={() => cancelSession(s)}
                        disabled={busyId === s.id}
                        class="rounded px-2.5 py-1 text-xs font-semibold text-amber-700
                               transition-colors hover:bg-amber-50 disabled:opacity-40"
                      >Cancel</button>
                    {/if}
                    <button
                      type="button"
                      onclick={() => removeSession(s)}
                      disabled={busyId === s.id}
                      class="rounded px-2.5 py-1 text-xs font-semibold text-red-500
                             transition-colors hover:bg-red-50 disabled:opacity-40"
                    >Delete</button>
                  </div>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </div>

  {#if data.sessions.total > perPage}
    <div class="flex items-center justify-between gap-3">
      <p class="text-xs text-brand-muted">
        {firstShown}–{lastShown} of {data.sessions.total}
      </p>
      <div class="flex gap-2">
        <button
          type="button"
          onclick={() => gotoPage(pageNo - 1)}
          disabled={pageNo <= 1}
          class="rounded-btn border border-brand-border px-4 py-1.5 text-sm font-semibold text-brand-text
                 transition-opacity hover:opacity-80 disabled:opacity-40"
        >
          Previous
        </button>
        <button
          type="button"
          onclick={() => gotoPage(pageNo + 1)}
          disabled={pageNo >= lastPage}
          class="rounded-btn border border-brand-border px-4 py-1.5 text-sm font-semibold text-brand-text
                 transition-opacity hover:opacity-80 disabled:opacity-40"
        >
          Next
        </button>
      </div>
    </div>
  {/if}
</div>
