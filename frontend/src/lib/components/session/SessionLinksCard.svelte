<script lang="ts">
  import type { SessionLinks } from "$lib/api";
  import { linkSession, unlinkSession } from "$lib/api";
  import { ApiError } from "$lib/api/errors";
  import { ikAvatar } from "$lib/imagekit";
  import { notify } from "$lib/notifications.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import { sessionLabel } from "$lib/session-links";

  let {
    links,
    sessionId,
    joinCode,
    canControl = false,
    onChanged = null,
  }: {
    /** The group this session is in; null when linked to nothing. */
    links: SessionLinks | null;
    sessionId: string;
    joinCode: string;
    /** The host (or an admin) may link and unlink. */
    canControl?: boolean;
    /** The group after a change — the page keeps it, and the board reads it. */
    onChanged?: ((links: SessionLinks | null) => void) | null;
  } = $props();

  let code = $state("");
  let busy = $state(false);

  const sessions = $derived(links?.sessions ?? []);
  const others = $derived(sessions.filter((s) => !s.current));
  const showStandings = $derived(links !== null && sessions.length > 1);

  const statusLabel: Record<string, string> = {
    lobby: "Lobby",
    running: "Live",
    paused: "Paused",
    finished: "Finished",
    cancelled: "Cancelled",
  };

  /** A session's name in the list and its column — twins get their join code. */
  const label = (i: number) => sessionLabel(sessions, i);

  /** Why a link was refused, in words — the codes the server answers with. */
  const LINK_ERRORS: Record<string, string> = {
    link_to_self: "that is this session's own code.",
    link_across_projects: "linked sessions run the same project — that session runs another one.",
    not_found: "no session has that code.",
    forbidden: "you can only link sessions you host.",
  };
  function linkError(err: unknown): string {
    if (err instanceof ApiError && err.code && LINK_ERRORS[err.code]) return LINK_ERRORS[err.code];
    return err instanceof Error ? err.message : String(err);
  }

  async function link(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    const target = code.trim().toUpperCase();
    if (!target || busy) return;
    busy = true;
    try {
      const next = await linkSession(sessionId, target, { fetch });
      code = "";
      notify.success(`Linked with ${target}.`);
      onChanged?.(next);
    } catch (err) {
      notify.error(`Failed to link: ${linkError(err)}`);
    } finally {
      busy = false;
    }
  }

  async function unlink(): Promise<void> {
    if (busy) return;
    if (!window.confirm("Unlink this session from the others?")) return;
    busy = true;
    try {
      await unlinkSession(sessionId, { fetch });
      notify.success("Unlinked.");
      onChanged?.(null);
    } catch (err) {
      notify.error(`Failed to unlink: ${err instanceof Error ? err.message : String(err)}`);
    } finally {
      busy = false;
    }
  }
</script>

<!--
  The sessions this one is linked with: a few rounds of a workshop, one
  brief for several rooms — read together. Each row leads to that session;
  the host links another by its join code, or takes this one out. The rooms
  run side by side, so the card lists every room's players by points as
  they play — the standings of the whole event — and calls them final once
  every session has ended.
  A session linked to nothing shows the card only to the host, as the way
  to start a group.
-->
{#if links !== null || canControl}
  <section
    class="mb-[20px] rounded-[8px] bg-white px-[24px] py-[20px]"
    aria-label="Linked sessions"
    data-testid="session-links"
  >
    <div class="mb-[14px] flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
      <h2 class="font-heading text-[16px] font-bold" style="color: #363636;">Linked sessions</h2>
      {#if sessions.length > 1}
        <span class="text-[13px] text-brand-muted">{sessions.length} sessions in this group</span>
      {/if}
    </div>

    {#if sessions.length > 1}
      <ol class="flex flex-col" data-testid="session-links-list">
        {#each sessions as s, i (s.id)}
          <li
            class="flex flex-col gap-[4px] py-[10px] sm:flex-row sm:items-baseline sm:gap-[12px]"
            class:border-t={i > 0}
            style="border-color: #eef3fb;"
            data-testid="session-link-{s.join_code}"
            data-current={s.current ? "true" : "false"}
          >
            <span
              class="flex h-[24px] w-[24px] shrink-0 items-center justify-center rounded-full text-[12px] font-bold tabular-nums
                {s.current ? 'bg-brand-blue text-white' : 'bg-[#e2ecfc] text-[#36547f]'}"
            >{i + 1}</span>
            <div class="min-w-0 flex-1">
              {#if s.current}
                <span class="text-[14px] font-semibold text-brand-text">{label(i)}</span>
              {:else}
                <a href="/s/{s.join_code}" class="text-[14px] font-semibold text-brand-text hover:text-brand-blue">
                  {label(i)}
                </a>
              {/if}
              <div class="mt-[2px] flex flex-wrap items-center gap-x-2 text-[13px] text-brand-muted">
                {#if label(i) !== s.join_code}
                  <span class="font-mono">{s.join_code}</span>
                  <span>·</span>
                {/if}
                <span>{s.player_count} {s.player_count === 1 ? "player" : "players"}</span>
              </div>
            </div>
            <span class="shrink-0 text-[12px] font-semibold uppercase tracking-wide text-brand-muted">
              {#if s.current}
                This session
              {:else}
                {statusLabel[s.status] ?? s.status}
              {/if}
            </span>
          </li>
        {/each}
      </ol>
    {:else if canControl}
      <p class="text-[13px] text-brand-muted" data-testid="session-links-empty">
        Link another of your sessions to read them as one event: the pages name each other, the
        board flips between them, and once every one has finished there is one standings for the lot.
      </p>
    {/if}

    {#if showStandings && links}
      <!-- The whole event's standings: every room's players in one list,
           sorted by what they earned, each with the session they played. -->
      <div class="mt-[18px]" data-testid="session-links-standings">
        <div class="mb-[10px] flex flex-wrap items-baseline justify-between gap-x-3">
          <h3 class="font-heading text-[14px] font-bold" style="color: #363636;">
            {links.all_finished ? "Final standings" : "Overall standings"}
          </h3>
          <span class="text-[12px] text-brand-muted" data-testid="session-links-standings-state">
            {links.all_finished ? "Every session has finished" : "Live — refreshed while the sessions play"}
          </span>
        </div>
        <table class="w-full text-left text-[13px]">
          <thead>
            <tr class="text-[11px] font-semibold uppercase tracking-wide text-brand-muted">
              <th class="py-[6px] pr-[8px]">#</th>
              <th class="py-[6px] pr-[12px]">Player</th>
              <th class="py-[6px] pr-[12px]">Session</th>
              <th class="py-[6px] text-right">Points</th>
            </tr>
          </thead>
          <tbody>
            {#each links.leaderboard as row, idx (row.user_id ?? `${row.display_name}-${idx}`)}
              <tr class="border-t" style="border-color: #eef3fb;" data-testid="session-links-row">
                <td class="py-[8px] pr-[8px] font-bold tabular-nums text-brand-muted">{idx + 1}</td>
                <td class="py-[8px] pr-[12px]">
                  <span class="inline-flex items-center gap-[8px]">
                    {#if row.avatar_url}
                      <img src={ikAvatar(row.avatar_url, 24)} alt="" class="h-[24px] w-[24px] rounded-full object-cover" />
                    {:else}
                      <span class="inline-flex h-[24px] w-[24px] items-center justify-center rounded-full bg-[#8fb4ec] text-[11px] font-bold text-white">
                        {row.display_name.charAt(0).toUpperCase()}
                      </span>
                    {/if}
                    {#if row.username}
                      <a href="/u/{row.username}" class="font-semibold text-brand-text hover:text-brand-blue">{row.display_name}</a>
                    {:else}
                      <span class="font-semibold text-brand-text">{row.display_name}</span>
                    {/if}
                  </span>
                </td>
                <td class="py-[8px] pr-[12px] text-brand-muted" data-testid="session-links-origin">
                  {#each row.per_session as part, j (part.session_id)}
                    {@const i = sessions.findIndex((s) => s.id === part.session_id)}
                    {#if j > 0}<span>{" · "}</span>{/if}
                    {#if i >= 0 && !sessions[i].current}
                      <a href="/s/{sessions[i].join_code}" class="hover:text-brand-blue">{label(i)}</a>
                    {:else}
                      <span>{i >= 0 ? label(i) : ""}</span>
                    {/if}
                    {#if row.per_session.length > 1}<span class="tabular-nums"> {part.points}</span>{/if}
                  {/each}
                </td>
                <td class="py-[8px] text-right font-bold tabular-nums {row.total_points >= 0 ? 'text-emerald-700' : 'text-brand-red'}">
                  {row.total_points}
                </td>
              </tr>
            {:else}
              <tr><td colspan="4" class="py-[8px] text-brand-muted">No results recorded.</td></tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}

    {#if canControl}
      <div class="mt-[16px] flex flex-wrap items-center gap-[10px] border-t pt-[14px]" style="border-color: #eef3fb;">
        <form class="flex items-center gap-[8px]" onsubmit={link} data-testid="session-links-form">
          <label class="sr-only" for="link-code-{joinCode}">Join code of the session to link</label>
          <input
            id="link-code-{joinCode}"
            class="h-[36px] w-[120px] rounded-[6px] border px-[10px] font-mono text-[13px] uppercase outline-none focus:border-brand-blue"
            style="border-color: #dce9fc;"
            placeholder="ABC123"
            maxlength="6"
            autocomplete="off"
            bind:value={code}
            disabled={busy}
          />
          <Button variant="outline" size="sm" type="submit" disabled={busy || code.trim().length !== 6}>Link session</Button>
        </form>
        {#if links !== null}
          <Button variant="ghost" size="sm" disabled={busy} onclick={unlink}>Unlink this session</Button>
        {/if}
      </div>
    {/if}
  </section>
{/if}
