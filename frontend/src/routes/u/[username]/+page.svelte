<script lang="ts">
  import { page } from '$app/state';
  import { ikAvatar } from '$lib/imagekit';
  import { formatDateUTC as formatDate, formatDateTimeUTC, formatTimeUTC } from '$lib/format';
  import {
    statusClass,
    statusLabels,
    sessionLinkLabel,
    sessionLinkShort,
  } from '$lib/session-status';
  import StatCard from '$lib/components/sessions/StatCard.svelte';

  let { data } = $props();

  // Which slice of the history is on screen, and where the rest of it is.
  const pageNo = $derived(data.sessions.page ?? 1);
  const perPage = $derived(data.sessions.per_page || 20);
  const pageCount = $derived(Math.max(1, Math.ceil(data.sessions.total / perPage)));
  const firstOnPage = $derived((pageNo - 1) * perPage + 1);
  const lastOnPage = $derived(firstOnPage + data.sessions.sessions.length - 1);

  function pageHref(n: number): string {
    const url = new URL(page.url);
    if (n <= 1) url.searchParams.delete('page');
    else url.searchParams.set('page', String(n));
    return url.pathname + url.search + '#sessions';
  }

  function getInitials(name: string): string {
    const parts = name.trim().split(/\s+/);
    if (parts.length >= 2) return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
    return name.slice(0, 2).toUpperCase();
  }

</script>

<svelte:head>
  <title>@{data.profile.username} — ololo.dev</title>
</svelte:head>

<div class="-mx-6 -mt-8 min-h-screen bg-brand-light-blue">
  <div class="mx-auto w-full max-w-[1206px] px-[18px] py-[88px]">

    <!-- Profile header -->
    <div class="flex flex-col gap-5 sm:flex-row sm:items-start sm:justify-between">
      <div class="flex items-center gap-6">
        {#if data.profile.avatar_url}
          <img
            src={ikAvatar(data.profile.avatar_url, 80)}
            alt={data.profile.display_name}
            class="h-20 w-20 rounded-full object-cover shadow-sm"
          />
        {:else}
          <div
            class="flex h-20 w-20 shrink-0 items-center justify-center rounded-full
                   bg-brand-blue/10 text-2xl font-bold text-brand-blue"
          >
            {getInitials(data.profile.display_name)}
          </div>
        {/if}
        <div>
          <h1 class="font-heading text-[28px] font-bold leading-tight text-brand-text">
            {data.profile.display_name}
          </h1>
          <p class="mt-0.5 text-sm text-brand-muted">@{data.profile.username}</p>
          {#if data.profile.joined_at}
            <p class="mt-1 text-xs text-brand-muted">
              Joined {formatDate(data.profile.joined_at)}
            </p>
          {/if}
        </div>
      </div>

      <div class="flex shrink-0 items-center gap-3">
        <StatCard label="Games" value={String(data.sessions.total)} />
      </div>
    </div>

    <!-- Sessions section -->
    <div class="mt-10 scroll-mt-6" id="sessions">
      <div class="mb-4 flex items-baseline gap-3">
        <h2 class="font-heading text-[20px] font-semibold text-brand-text">Sessions</h2>
        <span class="text-sm text-brand-muted">
          {data.sessions.total}
          {data.sessions.total === 1 ? 'session' : 'sessions'}
        </span>
      </div>

      <div class="overflow-hidden rounded-[8px] bg-white shadow-sm">
        {#if data.sessions.sessions.length === 0}
          <div class="flex flex-col items-center justify-center py-16 text-brand-muted">
            <svg
              width="40"
              height="40"
              viewBox="0 0 24 24"
              fill="none"
              class="mb-3 opacity-40"
              aria-hidden="true"
            >
              <rect x="3" y="3" width="18" height="18" rx="2" stroke="currentColor" stroke-width="2" />
              <path d="M3 9h18" stroke="currentColor" stroke-width="2" />
            </svg>
            <p class="text-sm">No sessions yet.</p>
          </div>
        {:else}
          <!-- One source of truth for every cell, rendered twice: phones get
               a card per session (eight columns cannot share 360px), md and
               up keeps the table. Both layouts stay in the DOM; CSS picks. -->
          {#snippet projectCell(session: (typeof data.sessions.sessions)[number])}
            <!-- The join code was the only link here, and it read as a
                 serial number rather than as a way into the session.
                 Now the cell names what was played and the code stays
                 as the identifier it is; the link out lives at the end
                 of the row, labelled with where it goes. -->
            <span class="flex min-w-0 flex-col leading-tight">
              {#if session.project_name}
                <a
                  href="/projects/{session.project_slug ?? session.project_id}"
                  class="truncate hover:text-brand-blue hover:underline"
                >{session.project_name}</a>
              {:else}
                <span class="truncate">{session.name}</span>
              {/if}
              <span class="font-mono text-xs font-normal text-brand-muted">
                {session.join_code}
              </span>
            </span>
          {/snippet}
          {#snippet statusPill(session: (typeof data.sessions.sessions)[number])}
            <span class="inline-flex items-center rounded-full px-2.5 py-0.5 text-[11px] font-semibold {statusClass(session.status)}">
              {statusLabels[session.status] ?? session.status}
            </span>
          {/snippet}
          {#snippet agentChip(session: (typeof data.sessions.sessions)[number])}
            {#if session.agent}
              <span
                class="inline-block rounded-full bg-brand-blue/10 px-2 py-0.5 text-xs font-semibold text-brand-blue"
                title={session.models?.length ? session.models.join(', ') : undefined}
              >{session.agent}</span>
            {:else}
              <span class="text-xs text-brand-muted">—</span>
            {/if}
          {/snippet}
          {#snippet scoreValue(session: (typeof data.sessions.sessions)[number])}
            {#if session.game_points != null}
              <span class={session.game_points < 0 ? 'text-red-600' : 'text-brand-text'}>
                {session.game_points}
              </span>
            {:else}
              <span class="font-normal text-brand-muted">—</span>
            {/if}
          {/snippet}
          {#snippet placeValue(session: (typeof data.sessions.sessions)[number])}
            <!-- "#1 of 1" was true on almost every row and told the reader
                 nothing: most sessions are played alone. A placement is only
                 a placement when someone else was in the room. -->
            {#if session.participant_count <= 1}
              <span class="font-normal text-brand-muted">Solo</span>
            {:else if session.placement != null}
              <span class="text-brand-text">#{session.placement}</span>
              <span class="font-normal text-brand-muted">of {session.participant_count}</span>
            {:else}
              <!-- Cancelled before anyone placed: the dash is the answer, and
                   the field size is still worth knowing. -->
              <span class="font-normal text-brand-muted">— of {session.participant_count}</span>
            {/if}
          {/snippet}
          {#snippet whenValue(session: (typeof data.sessions.sessions)[number])}
            <!-- One column, two lines: the date is what a reader scans for and
                 the time is detail. On one line it was the widest column on
                 the page. -->
            <span class="flex flex-col leading-tight" title={formatDateTimeUTC(session.session_datetime)}>
              <span class="whitespace-nowrap text-brand-text">{formatDate(session.session_datetime)}</span>
              <span class="whitespace-nowrap text-xs text-brand-muted">{formatTimeUTC(session.session_datetime)}</span>
            </span>
          {/snippet}
          {#snippet sessionLink(session: (typeof data.sessions.sessions)[number])}
            <!-- Short on screen, full to a screen reader: the long phrase on
                 every row is what pushed this column off the table. -->
            <a
              href="/s/{session.join_code}"
              aria-label={sessionLinkLabel(session.status)}
              title={sessionLinkLabel(session.status)}
              class="inline-block whitespace-nowrap rounded px-3 py-2 text-xs font-semibold
                     text-brand-blue transition-colors hover:bg-brand-blue/10 md:py-1"
            >{sessionLinkShort(session.status)} →</a>
          {/snippet}

          <ul class="divide-y divide-brand-border/60 md:hidden" data-testid="profile-sessions-cards">
            {#each data.sessions.sessions as session, i (i)}
              <li class="p-4">
                <div class="flex items-start justify-between gap-3">
                  <span class="min-w-0 text-sm font-medium text-brand-text">
                    {@render projectCell(session)}
                  </span>
                  {@render statusPill(session)}
                </div>
                <div class="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-brand-muted">
                  <span class="whitespace-nowrap">{formatDateTimeUTC(session.session_datetime)}</span>
                  {#if session.agent}
                    {@render agentChip(session)}
                  {/if}
                </div>
                <div class="mt-3 flex items-end justify-between gap-3">
                  <dl class="flex gap-5">
                    <div>
                      <dt
                        class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted"
                        title="Points scored inside the session"
                      >Score</dt>
                      <dd class="text-sm font-semibold tabular-nums">{@render scoreValue(session)}</dd>
                    </div>
                    <div>
                      <dt class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Place</dt>
                      <dd class="whitespace-nowrap text-sm font-semibold tabular-nums">{@render placeValue(session)}</dd>
                    </div>
                  </dl>
                  {@render sessionLink(session)}
                </div>
              </li>
            {/each}
          </ul>

          <!-- The card rounds its corners with overflow-hidden, which silently
               clipped the last column off the page when the row grew. The
               table scrolls inside it now instead of being cut. -->
          <div class="hidden overflow-x-auto md:block">
          <table class="w-full min-w-[720px]">
            <thead>
              <tr class="border-b border-brand-border bg-brand-light-blue/40">
                <th class="px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted lg:px-6">
                  Project
                </th>
                <th class="px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted lg:px-6">
                  Status
                </th>
                <th class="px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted lg:px-6">
                  Date
                </th>
                <th class="px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted lg:px-6">
                  Agent
                </th>
                <th
                  class="px-4 py-3 text-right text-[11px] font-semibold uppercase tracking-wider text-brand-muted lg:px-6"
                  title="Points scored inside the session"
                >
                  Score
                </th>
                <th
                  class="px-4 py-3 text-right text-[11px] font-semibold uppercase tracking-wider text-brand-muted lg:px-6"
                  title="Finishing position among the players in that session"
                >
                  Place
                </th>
                <th class="px-4 py-3 text-right text-[11px] font-semibold uppercase tracking-wider text-brand-muted lg:px-6">
                  Session
                </th>
              </tr>
            </thead>
            <tbody>
              {#each data.sessions.sessions as session, i (i)}
                <tr class="border-b border-brand-border/60 last:border-0 transition-colors hover:bg-brand-light-blue/30">
                  <td class="px-4 py-4 text-sm font-medium text-brand-text lg:px-6">
                    {@render projectCell(session)}
                  </td>
                  <td class="px-4 py-4 lg:px-6">
                    {@render statusPill(session)}
                  </td>
                  <td class="px-4 py-4 text-sm lg:px-6">
                    {@render whenValue(session)}
                  </td>
                  <td class="px-4 py-4 text-sm lg:px-6">
                    {@render agentChip(session)}
                  </td>
                  <td class="px-4 py-4 text-right text-sm font-semibold tabular-nums lg:px-6">
                    {@render scoreValue(session)}
                  </td>
                  <td class="whitespace-nowrap px-4 py-4 text-right text-sm font-semibold tabular-nums lg:px-6">
                    {@render placeValue(session)}
                  </td>
                  <td class="whitespace-nowrap px-4 py-4 text-right lg:px-6">
                    {@render sessionLink(session)}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
          </div>

          {#if pageCount > 1}
            <!-- The heading counts every session ever played; this counts the
                 ones actually on the page, and offers the rest. Plain links,
                 so a page of history can be linked to and comes back the same
                 on reload. -->
            <nav
              class="flex flex-wrap items-center justify-between gap-3 border-t border-brand-border/60 px-4 py-3 lg:px-6"
              aria-label="Sessions pages"
              data-testid="sessions-pagination"
            >
              <span class="text-xs text-brand-muted">
                {firstOnPage}–{lastOnPage} of {data.sessions.total}
              </span>
              <span class="flex items-center gap-1">
                {#if pageNo > 1}
                  <a
                    href={pageHref(pageNo - 1)}
                    class="rounded px-3 py-1 text-xs font-semibold text-brand-blue transition-colors hover:bg-brand-blue/10"
                  >← Newer</a>
                {:else}
                  <span class="px-3 py-1 text-xs font-semibold text-brand-muted/60">← Newer</span>
                {/if}
                <span class="px-2 text-xs text-brand-muted">Page {pageNo} of {pageCount}</span>
                {#if pageNo < pageCount}
                  <a
                    href={pageHref(pageNo + 1)}
                    class="rounded px-3 py-1 text-xs font-semibold text-brand-blue transition-colors hover:bg-brand-blue/10"
                  >Older →</a>
                {:else}
                  <span class="px-3 py-1 text-xs font-semibold text-brand-muted/60">Older →</span>
                {/if}
              </span>
            </nav>
          {/if}
        {/if}
      </div>
    </div>

  </div>
</div>
