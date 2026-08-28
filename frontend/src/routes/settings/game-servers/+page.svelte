<script lang="ts">
  import { invalidateAll } from '$app/navigation';
  import { onMount } from 'svelte';
  import { browser } from '$app/environment';
  import { notify } from '$lib/notifications.svelte';
  import { getToken } from '$lib/auth';
  import { Drawer } from 'vaul-svelte';
  import {
    getGameServers,
    patchGameServer,
    ApiError,
    type GameServerDto,
  } from '$lib/api';
  import type { ZmqEvent } from '$lib/types/arena';

  let servers = $state<GameServerDto[]>([]);
  let loading = $state(true);
  let editingId = $state<string | null>(null);
  let editDisplayName = $state('');
  let saving = $state<string | null>(null);

  let drawerOpen = $state(false);
  let eventStreamConnected = $state(false);
  let eventLog = $state<ZmqEvent[]>([]);
  let eventFilter = $state('');
  let maxEventLogSize = 200;
  let ws: WebSocket | null = null;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let reconnectDelay = 1000;

  onMount(() => {
    loadServers();
    return () => {
      closeEventStream();
      if (reconnectTimer) clearTimeout(reconnectTimer);
    };
  });

  async function loadServers() {
    loading = true;
    try {
      servers = await getGameServers();
    } catch {
      notify.error('Failed to load game servers.', 'Game Servers');
    } finally {
      loading = false;
    }
  }

  function formatRelativeTime(iso: string): string {
    const now = Date.now();
    const then = new Date(iso).getTime();
    const diffSec = Math.floor((now - then) / 1000);
    if (diffSec < 5) return 'just now';
    if (diffSec < 60) return `${diffSec}s ago`;
    if (diffSec < 3600) return `${Math.floor(diffSec / 60)}m ago`;
    if (diffSec < 86400) return `${Math.floor(diffSec / 3600)}h ago`;
    return `${Math.floor(diffSec / 86400)}d ago`;
  }

  function statusColor(status: string): string {
    switch (status) {
      case 'active': return 'bg-emerald-100 text-emerald-700';
      case 'draining': return 'bg-amber-100 text-amber-700';
      case 'inactive': return 'bg-red-100 text-red-700';
      default: return 'bg-gray-100 text-gray-700';
    }
  }

  function startEditDisplayName(server: GameServerDto) {
    editingId = server.id;
    editDisplayName = server.display_name ?? '';
  }

  function cancelEditDisplayName() {
    editingId = null;
    editDisplayName = '';
  }

  async function saveDisplayName(server: GameServerDto) {
    if (editDisplayName === '') {
      saving = server.id;
      try {
        await patchGameServer(server.id, { clear_display_name: true });
        await invalidateAll();
        notify.success('Display name cleared.', 'Game Servers');
      } catch {
        notify.error('Failed to clear display name.', 'Game Servers');
      } finally {
        saving = null;
        editingId = null;
      }
    } else {
      saving = server.id;
      try {
        await patchGameServer(server.id, { display_name: editDisplayName });
        await invalidateAll();
        notify.success('Display name updated.', 'Game Servers');
      } catch {
        notify.error('Failed to update display name.', 'Game Servers');
      } finally {
        saving = null;
        editingId = null;
      }
    }
  }

  async function changeStatus(server: GameServerDto, newStatus: string) {
    const oldStatus = server.status;
    saving = server.id;
    try {
      await patchGameServer(server.id, { status: newStatus });
      await invalidateAll();
      notify.success(`Status changed to ${newStatus}.`, 'Game Servers');
    } catch (err) {
      if (err instanceof ApiError && err.code === 'server_has_active_sessions') {
        const count = (err.body as { active_sessions?: number })?.active_sessions ?? server.active_sessions;
        notify.error(
          `Cannot set to inactive — server has ${count} active session${count === 1 ? '' : 's'}. Set to draining first.`,
          'Game Servers',
        );
      } else {
        notify.error('Failed to update status.', 'Game Servers');
      }
    } finally {
      saving = null;
    }
  }

  function openEventStream() {
    drawerOpen = true;
    connectEventStream();
  }

  function closeEventStream() {
    drawerOpen = false;
    if (ws) {
      ws.close();
      ws = null;
    }
    eventStreamConnected = false;
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
  }

  function connectEventStream() {
    if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) return;

    const proto = typeof window !== 'undefined' && window.location.protocol === 'https:' ? 'wss' : 'ws';
    const host = typeof window !== 'undefined' ? window.location.host : 'localhost:8080';
    const token = getToken() ?? '';

    const url = `${proto}://${host}/ws/admin/zmq?token=${encodeURIComponent(token)}`;
    console.log('[ZMQ Event Stream] Connecting', 'token:', token ? 'present' : 'missing');

    try {
      ws = new WebSocket(url);
    } catch {
      eventStreamConnected = false;
      return;
    }

    ws.onopen = () => {
      eventStreamConnected = true;
      reconnectDelay = 1000;
      console.log('[ZMQ Event Stream] Connected');
    };

    ws.onmessage = (ev) => {
      if (typeof ev.data !== 'string') return;
      try {
        const event: ZmqEvent = JSON.parse(ev.data);
        eventLog = [event, ...eventLog].slice(0, maxEventLogSize);
      } catch {
        // ignore malformed
      }
    };

    ws.onclose = () => {
      eventStreamConnected = false;
      console.log('[ZMQ Event Stream] Disconnected');
      if (drawerOpen) {
        reconnectTimer = setTimeout(() => {
          reconnectDelay = Math.min(reconnectDelay * 2, 30000);
          connectEventStream();
        }, reconnectDelay);
      }
    };

    ws.onerror = (ev) => {
      console.error('[ZMQ Event Stream] WebSocket error', ev);
      ws?.close();
    };
  }

  function clearEventLog() {
    eventLog = [];
  }

  function eventVariantBadge(event: ZmqEvent): string {
    switch (event.type) {
      case 'session_timer': return 'bg-blue-100 text-blue-700';
      case 'session_status': return 'bg-purple-100 text-purple-700';
      case 'player_join': return 'bg-emerald-100 text-emerald-700';
      case 'player_leave': return 'bg-red-100 text-red-700';
      case 'score_change': return 'bg-amber-100 text-amber-700';
    }
  }

  let filteredEvents = $derived(
    eventFilter
      ? eventLog.filter(e => e.join_code.toLowerCase().includes(eventFilter.toLowerCase()) || e.type.includes(eventFilter.toLowerCase()))
      : eventLog
  );
</script>

<div class="mt-8">
  <div class="mb-4 flex flex-wrap items-center justify-between gap-3">
    <div>
      <h2 class="font-heading text-[20px] font-semibold text-brand-text">Game Servers</h2>
      <p class="mt-0.5 text-sm text-brand-muted">
        {loading ? 'Loading…' : `${servers.length} ${servers.length === 1 ? 'server' : 'servers'} registered.`}
        Game servers register automatically via heartbeat.
      </p>
    </div>
    <button
      type="button"
      onclick={openEventStream}
      class="inline-flex shrink-0 items-center gap-1.5 rounded-[6px] border border-brand-border bg-white px-3 py-1.5 text-sm font-medium text-brand-text shadow-sm transition-colors hover:bg-brand-light-blue"
    >
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z"/>
      </svg>
      Event Stream
    </button>
  </div>

  {#if loading && servers.length === 0}
    <div class="flex items-center justify-center py-12">
      <div class="text-sm text-brand-muted">Loading game servers…</div>
    </div>
  {:else if servers.length === 0}
    <div class="flex flex-col items-center justify-center py-16 text-brand-muted">
      <svg width="36" height="36" viewBox="0 0 24 24" fill="none" class="mb-3 opacity-40" aria-hidden="true">
        <rect x="2" y="2" width="20" height="8" rx="2" stroke="currentColor" stroke-width="2"/>
        <rect x="2" y="14" width="20" height="8" rx="2" stroke="currentColor" stroke-width="2"/>
        <circle cx="6" cy="6" r="1" fill="currentColor"/>
        <circle cx="6" cy="18" r="1" fill="currentColor"/>
      </svg>
      <p class="text-sm">No game servers registered yet. They will appear here automatically.</p>
    </div>
  {:else}
    <div class="rounded-[8px] bg-white shadow-sm">
      <!-- Cards below `xl`, matching the admin sessions list: the settings
           column is ~940px wide, which seven columns do not fit. -->
      <ul class="divide-y divide-brand-border/60 xl:hidden">
        {#each servers as server (server.id)}
          <li class="flex flex-col gap-2.5 p-4">
            <div class="flex items-start justify-between gap-3">
              <div class="min-w-0 flex-1">
                {#if editingId === server.id}
                  <div class="flex flex-col gap-2">
                    <div oninput={(e) => (editDisplayName = (e.target as HTMLInputElement).value)}>
                      <input
                        type="text"
                        value={editDisplayName}
                        placeholder="Enter display name…"
                        class="w-full rounded-[6px] border border-brand-blue px-2 py-1 text-sm
                               text-brand-text focus:outline-none focus:ring-2 focus:ring-brand-blue"
                      />
                    </div>
                    <div class="flex gap-2">
                      <button
                        type="button"
                        disabled={saving === server.id}
                        onclick={() => saveDisplayName(server)}
                        class="rounded px-2 py-1 text-xs font-semibold text-brand-blue
                               transition-colors hover:bg-brand-blue/10 disabled:opacity-40"
                      >
                        {saving === server.id ? '…' : 'Save'}
                      </button>
                      <button
                        type="button"
                        onclick={cancelEditDisplayName}
                        class="rounded px-2 py-1 text-xs font-semibold text-brand-muted
                               transition-colors hover:text-brand-text"
                      >
                        Cancel
                      </button>
                    </div>
                  </div>
                {:else}
                  <button
                    type="button"
                    onclick={() => startEditDisplayName(server)}
                    class="group flex min-w-0 max-w-full items-center gap-1 text-sm text-brand-text hover:text-brand-blue transition-colors"
                  >
                    {#if server.display_name}
                      <span class="truncate">{server.display_name}</span>
                    {:else}
                      <span class="truncate italic text-brand-muted">{server.url}</span>
                    {/if}
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" class="shrink-0 opacity-0 group-hover:opacity-60 transition-opacity" aria-hidden="true">
                      <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
                      <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
                    </svg>
                  </button>
                {/if}
              </div>
              <select
                value={server.status}
                onchange={(e) => changeStatus(server, (e.target as HTMLSelectElement).value)}
                disabled={saving === server.id}
                class="shrink-0 rounded-[6px] border border-brand-border px-2 py-1 text-sm
                       {statusColor(server.status)} focus:outline-none focus:ring-2 focus:ring-brand-blue
                       disabled:opacity-50 disabled:cursor-not-allowed"
              >
                <option value="active">active</option>
                <option value="draining">draining</option>
                <option value="inactive">inactive</option>
              </select>
            </div>

            <dl class="grid grid-cols-[5.5rem_minmax(0,1fr)] items-center gap-x-3 gap-y-1.5 text-xs text-brand-muted">
              <dt class="font-semibold uppercase tracking-wider text-brand-muted/70">URL</dt>
              <dd class="min-w-0 break-all font-mono">{server.url}</dd>
              <dt class="font-semibold uppercase tracking-wider text-brand-muted/70">ZMQ</dt>
              <dd class="min-w-0 break-all font-mono">
                {#if server.zmq_url}
                  <span class="text-emerald-600">{server.zmq_url}</span>
                {:else}
                  <span class="text-gray-400">—</span>
                {/if}
              </dd>
              <dt class="font-semibold uppercase tracking-wider text-brand-muted/70">Capacity</dt>
              <dd class="text-brand-text">{server.capacity}</dd>
              <dt class="font-semibold uppercase tracking-wider text-brand-muted/70">Active</dt>
              <dd class="text-brand-text">{server.active_sessions}</dd>
              <dt class="font-semibold uppercase tracking-wider text-brand-muted/70">Heartbeat</dt>
              <dd>{formatRelativeTime(server.last_heartbeat)}</dd>
            </dl>
          </li>
        {/each}
      </ul>

      <!-- table-fixed with flexible URL columns: the two mono columns absorb
           whatever width is left and truncate, so the table fits the ~940px
           settings column without a horizontal scrollbar. -->
      <div class="hidden xl:block">
        <table class="w-full table-fixed">
          <thead>
            <tr class="border-b border-brand-border bg-brand-light-blue/40">
              <th class="w-[20%] py-3 pl-6 pr-4 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
                Display Name
              </th>
              <th class="px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
                URL
              </th>
              <th class="px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
                ZMQ
              </th>
              <th class="w-[76px] px-4 py-3 text-center text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
                Capacity
              </th>
              <th class="w-[64px] px-4 py-3 text-center text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
                Active
              </th>
              <th class="w-[108px] px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
                Status
              </th>
              <th class="w-[104px] py-3 pl-4 pr-6 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
                Last Heartbeat
              </th>
            </tr>
          </thead>
          <tbody>
            {#each servers as server (server.id)}
              <tr class="border-b border-brand-border/60 last:border-0 hover:bg-brand-light-blue/30 transition-colors">
                <td class="py-4 pl-6 pr-4">
                  {#if editingId === server.id}
                    <div class="flex items-center gap-2">
                      <div class="min-w-0 flex-1" oninput={(e) => (editDisplayName = (e.target as HTMLInputElement).value)}>
                        <input
                          type="text"
                          value={editDisplayName}
                          placeholder="Enter display name…"
                          class="w-full rounded-[6px] border border-brand-blue px-2 py-1 text-sm
                                 text-brand-text focus:outline-none focus:ring-2 focus:ring-brand-blue"
                        />
                      </div>
                      <button
                        type="button"
                        disabled={saving === server.id}
                        onclick={() => saveDisplayName(server)}
                        class="rounded px-2 py-1 text-xs font-semibold text-brand-blue
                               transition-colors hover:bg-brand-blue/10 disabled:opacity-40"
                      >
                        {saving === server.id ? '…' : 'Save'}
                      </button>
                      <button
                        type="button"
                        onclick={cancelEditDisplayName}
                        class="rounded px-2 py-1 text-xs font-semibold text-brand-muted
                               transition-colors hover:text-brand-text"
                      >
                        Cancel
                      </button>
                    </div>
                  {:else}
                    <button
                      type="button"
                      onclick={() => startEditDisplayName(server)}
                      class="group flex min-w-0 max-w-full items-center gap-1 text-sm text-brand-text hover:text-brand-blue transition-colors"
                    >
                      {#if server.display_name}
                        <span class="truncate" title={server.display_name}>{server.display_name}</span>
                      {:else}
                        <span class="truncate italic text-brand-muted" title={server.url}>{server.url}</span>
                      {/if}
                      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" class="shrink-0 opacity-0 group-hover:opacity-60 transition-opacity" aria-hidden="true">
                        <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
                        <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
                      </svg>
                    </button>
                  {/if}
                </td>
                <td class="px-4 py-4 text-sm font-mono text-brand-muted">
                  <span class="block truncate" title={server.url}>{server.url}</span>
                </td>
                <td class="px-4 py-4 text-sm font-mono text-brand-muted">
                  {#if server.zmq_url}
                    <span class="block truncate text-emerald-600" title={server.zmq_url}>{server.zmq_url}</span>
                  {:else}
                    <span class="text-gray-400">—</span>
                  {/if}
                </td>
                <td class="px-4 py-4 text-center text-sm text-brand-text">
                  {server.capacity}
                </td>
                <td class="px-4 py-4 text-center text-sm text-brand-text">
                  {server.active_sessions}
                </td>
                <td class="px-4 py-4">
                  <select
                    value={server.status}
                    onchange={(e) => changeStatus(server, (e.target as HTMLSelectElement).value)}
                    disabled={saving === server.id}
                    class="rounded-[6px] border border-brand-border px-2 py-1 text-sm
                           {statusColor(server.status)} focus:outline-none focus:ring-2 focus:ring-brand-blue
                           disabled:opacity-50 disabled:cursor-not-allowed"
                  >
                    <option value="active">active</option>
                    <option value="draining">draining</option>
                    <option value="inactive">inactive</option>
                  </select>
                </td>
                <td class="whitespace-nowrap py-4 pl-4 pr-6 text-sm text-brand-muted">
                  {formatRelativeTime(server.last_heartbeat)}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    </div>
  {/if}
</div>

{#if browser}
  <Drawer.Root
    open={drawerOpen}
    onOpenChange={(v) => { if (!v) closeEventStream(); }}
  >
    <Drawer.Portal>
      <Drawer.Overlay class="fixed inset-0 bg-black/40" />
    <Drawer.Content class="fixed inset-x-0 bottom-0 mt-24 flex h-[70vh] flex-col rounded-t-[10px] bg-white shadow-xl">
      <div class="flex flex-wrap items-center justify-between gap-3 border-b border-gray-200 px-4 py-4 sm:px-6">
        <div class="flex items-center gap-3">
          <Drawer.Title class="text-lg font-semibold text-gray-900">
            ZMQ Event Stream
          </Drawer.Title>
          <span class="inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-xs font-medium
            {eventStreamConnected ? 'bg-emerald-100 text-emerald-700' : 'bg-red-100 text-red-700'}">
            <span class="h-1.5 w-1.5 rounded-full {eventStreamConnected ? 'bg-emerald-500' : 'bg-red-500'}"></span>
            {eventStreamConnected ? 'Live' : 'Disconnected'}
          </span>
        </div>
        <div class="flex w-full flex-wrap items-center gap-2 sm:w-auto">
          <input
            type="text"
            placeholder="Filter by join code or type…"
            bind:value={eventFilter}
            class="min-w-0 flex-1 rounded-[6px] border border-gray-300 px-3 py-1.5 text-sm text-gray-900
                   placeholder:text-gray-400 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500
                   sm:flex-none"
          />
          <button
            type="button"
            onclick={clearEventLog}
            class="shrink-0 rounded-[6px] border border-gray-300 px-3 py-1.5 text-sm font-medium text-gray-700
                   hover:bg-gray-50"
          >
            Clear
          </button>
          <button
            type="button"
            onclick={closeEventStream}
            class="shrink-0 rounded-[6px] border border-gray-300 px-3 py-1.5 text-sm font-medium text-gray-700
                   hover:bg-gray-50"
          >
            Close
          </button>
        </div>
      </div>
      <div class="flex-1 overflow-y-auto px-4 py-2 sm:px-6">
        {#if filteredEvents.length === 0}
          <div class="flex h-full items-center justify-center text-sm text-gray-400">
            {#if eventStreamConnected}
              Waiting for events…
            {:else}
              Not connected to game server event stream.
            {/if}
          </div>
        {:else}
          <table class="w-full text-sm">
            <thead class="sticky top-0 bg-white">
              <tr class="border-b text-left text-xs font-medium uppercase tracking-wider text-gray-500">
                <th class="py-2 pr-3">Time</th>
                <th class="py-2 pr-3">Type</th>
                <th class="py-2 pr-3">Join Code</th>
                <th class="py-2">Details</th>
              </tr>
            </thead>
            <tbody>
              {#each filteredEvents as event, i (i)}
                <tr class="border-b border-gray-100 last:border-0">
                  <td class="py-1.5 pr-3 text-xs text-gray-400 whitespace-nowrap">
                    {new Date().toLocaleTimeString()}
                  </td>
                  <td class="py-1.5 pr-3">
                    <span class="inline-block rounded px-1.5 py-0.5 text-xs font-medium {eventVariantBadge(event)}">
                      {event.type.replace('_', ' ')}
                    </span>
                  </td>
                  <td class="py-1.5 pr-3 font-mono text-xs font-semibold text-gray-700">
                    {event.join_code}
                  </td>
                  <td class="py-1.5 text-xs text-gray-600">
                    {#if event.type === 'session_timer'}
                      {event.phase} · {event.seconds_remaining}s · v{event.version}
                    {:else if event.type === 'session_status'}
                      {event.status} · v{event.version}
                    {:else if event.type === 'player_join'}
                      {event.display_name} · v{event.version}
                    {:else if event.type === 'player_leave'}
                      v{event.version}
                    {:else if event.type === 'score_change'}
                      Δ{event.delta >= 0 ? '+' : ''}{event.delta} → {event.total} · v{event.version}
                    {/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}
      </div>
      </Drawer.Content>
    </Drawer.Portal>
  </Drawer.Root>
{/if}