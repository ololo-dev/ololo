<script lang="ts">
  import { browser } from "$app/environment";
  import { marked } from "marked";
  import DOMPurify from "isomorphic-dompurify";
  import { Collapsible, CollapsibleTrigger, CollapsibleContent } from "$lib/components/ui/collapsible";
  import { formatDateTimeUTC, formatTimeUTC } from "$lib/format";

  let {
    joinCode,
    projectName,
    projectSlug,
    projectId,
    badge,
    showCopy,
    showShare = false,
    copied,
    onCopy,
    description,
    startedAt = null,
    finishedAt = null,
    createdAt = null,
  }: {
    joinCode: string;
    projectName?: string | null;
    projectSlug?: string | null;
    projectId?: string | null;
    /// "judging" = the timer has stopped but judges still owe verdicts —
    /// the session must not claim "Complete" while the standings can move.
    badge: "lobby" | "live" | "paused" | "judging" | "complete";
    showCopy: boolean;
    /** Finished sessions: offer a "copy results link" instead of the join flow. */
    showShare?: boolean;
    copied: boolean;
    onCopy: () => void;
    description?: string | null;
    startedAt?: string | null;
    finishedAt?: string | null;
    createdAt?: string | null;
  } = $props();

  let descOpen = $state(false);

  // "Copy invite link" — the shareable web entry into this session: the
  // public session dashboard (/s/<code>), which shows the project, status,
  // roster and the `ololo join <code>` command an invitee needs. The CLI
  // command below is for the player themselves.
  let inviteCopied = $state(false);
  let inviteTimer: ReturnType<typeof setTimeout> | undefined;
  async function copyInviteLink() {
    if (!browser) return;
    try {
      await navigator.clipboard.writeText(`${window.location.origin}/s/${joinCode}`);
      inviteCopied = true;
      clearTimeout(inviteTimer);
      inviteTimer = setTimeout(() => (inviteCopied = false), 1500);
    } catch {
      // clipboard unavailable
    }
  }

  // "Copy results link" — a finished session's report is worth bragging
  // about, but there was no affordance to share it (audit UI-M5). Copies the
  // spectator URL, which anyone can open.
  let shareCopied = $state(false);
  let shareTimer: ReturnType<typeof setTimeout> | undefined;
  async function copyShareLink() {
    if (!browser) return;
    try {
      await navigator.clipboard.writeText(`${window.location.origin}/s/${joinCode}`);
      shareCopied = true;
      clearTimeout(shareTimer);
      shareTimer = setTimeout(() => (shareCopied = false), 1500);
    } catch {
      // clipboard unavailable
    }
  }

  // When the session ran, not only how long it lasted. UTC with the label,
  // like every other surface: a locale/timezone-dependent string would render
  // one value on the server and another after hydration (UI-M9).
  const dateLine = $derived.by(() => {
    if (startedAt) {
      const start = formatDateTimeUTC(startedAt);
      if (finishedAt) {
        const sameDay =
          new Date(startedAt).toISOString().slice(0, 10) ===
          new Date(finishedAt).toISOString().slice(0, 10);
        return `${start} – ${sameDay ? formatTimeUTC(finishedAt) : formatDateTimeUTC(finishedAt)}`;
      }
      return `started ${start}`;
    }
    return createdAt ? `created ${formatDateTimeUTC(createdAt)}` : null;
  });

  const descriptionHtml = $derived(
    description
      ? DOMPurify.sanitize(marked.parse(description) as string, {
          USE_PROFILES: { html: true },
        })
      : null,
  );

  const projectHref = $derived(
    projectSlug
      ? `/projects/${projectSlug}`
      : projectId
        ? `/projects/${projectId}`
        : "#",
  );

  const badgeClass = $derived(
    badge === "live"
      ? "background: rgba(251,52,28,0.1); color: #fb341c;"
      : badge === "paused"
        ? "background: rgba(143,180,236,0.15); color: #8fb4ec;"
        : badge === "complete"
          ? "background: rgba(107,229,151,0.15); color: #6be597;"
          : badge === "judging"
            ? "background: rgba(245,183,49,0.15); color: #c8930a;"
            : "background: rgba(143,180,236,0.15); color: #8fb4ec;",
  );
</script>

<div class="mb-[20px] overflow-hidden rounded-[8px] bg-white px-[24px] py-[20px]">
  <div class="mb-[16px] flex flex-wrap items-center gap-[12px]">
    <h1 class="font-heading text-[28px] font-bold leading-tight" style="color: #363636;">
      {joinCode}
    </h1>

    {#if badge === "live"}
      <span
        class="flex items-center gap-[6px] rounded-full px-[10px] py-[3px] text-[11px] font-semibold"
        style={badgeClass}
      >
        <span
          class="inline-block h-[6px] w-[6px] rounded-full"
          style="background: #fb341c; animation: pulse 1.5s ease-in-out infinite;"
        ></span>
        Live
      </span>
    {:else}
      <span
        class="rounded-full px-[10px] py-[3px] text-[11px] font-semibold"
        style={badgeClass}
      >{badge === "lobby"
          ? "Lobby"
          : badge === "paused"
            ? "Paused"
            : badge === "judging"
              ? "Judging"
              : "Complete"}</span>
    {/if}

    {#if projectName}
      <a
        href={projectHref}
        class="text-[14px] font-medium hover:underline"
        style="color: #4a90e2;"
      >{projectName}</a>
    {/if}

    {#if dateLine}
      <span class="ml-auto text-[12px] tabular-nums" style="color: #8fb4ec;" data-testid="session-date">
        {dateLine}
      </span>
    {/if}
  </div>

  {#if showCopy}
    <div
      class="flex items-center justify-between rounded-[4px] font-mono text-sm text-white"
      style="background: #36547f; border-left: 8px solid #fb341c;"
    >
      <code class="px-[20px] py-[12px]">ololo join {joinCode}</code>
      <button
        onclick={onCopy}
        title="Copy command"
        class="mr-[6px] flex shrink-0 items-center gap-[5px] rounded-[4px] px-[10px] py-[6px] text-[11px] font-sans font-semibold transition-all"
        style="color: {copied ? '#6be597' : 'rgba(255,255,255,0.65)'}; background: {copied ? 'rgba(107,229,151,0.12)' : 'transparent'};"
      >
        {#if copied}
          <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"></polyline></svg>
          Copied!
        {:else}
          <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="8" height="4" x="8" y="2" rx="1" ry="1"></rect><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"></path></svg>
          Copy
        {/if}
      </button>
    </div>
    <div class="mt-[8px] flex items-center gap-[8px]">
      <button
        onclick={copyInviteLink}
        data-testid="copy-invite-link"
        class="flex items-center gap-[5px] text-[12px] font-semibold transition-colors"
        style="color: {inviteCopied ? '#6be597' : '#4a90e2'};"
      >
        {#if inviteCopied}
          <svg xmlns="http://www.w3.org/2000/svg" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"></polyline></svg>
          Invite link copied!
        {:else}
          <svg xmlns="http://www.w3.org/2000/svg" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"></path><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"></path></svg>
          Copy invite link
        {/if}
      </button>
      <span class="text-[12px]" style="color: #8fb4ec;">
        — send it to invite another player
      </span>
    </div>
  {/if}

  {#if showShare}
    <div class="mt-[4px] flex items-center gap-[8px]">
      <button
        onclick={copyShareLink}
        data-testid="copy-results-link"
        class="flex items-center gap-[5px] text-[12px] font-semibold transition-colors"
        style="color: {shareCopied ? '#6be597' : '#4a90e2'};"
      >
        {#if shareCopied}
          <svg xmlns="http://www.w3.org/2000/svg" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"></polyline></svg>
          Results link copied!
        {:else}
          <svg xmlns="http://www.w3.org/2000/svg" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"></path><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"></path></svg>
          Copy results link
        {/if}
      </button>
      <span class="text-[12px]" style="color: #8fb4ec;">
        — share the final standings
      </span>
    </div>
  {/if}

  {#if descriptionHtml && browser}
    <div class="mt-[16px]">
      <Collapsible open={descOpen} onOpenChange={(v) => (descOpen = v)}>
        <CollapsibleTrigger
          class="flex items-center gap-[6px] text-[13px] font-semibold transition-opacity hover:opacity-70"
          style="color: #4a90e2;"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"
            style="transform: rotate({descOpen ? 90 : 0}deg); transition: transform 0.15s ease;"
          ><polyline points="9 18 15 12 9 6"></polyline></svg>
          Description
        </CollapsibleTrigger>
        <CollapsibleContent>
          <div class="prose prose-sm mt-[12px] max-w-none" style="color: #4a5568;">
            {@html descriptionHtml}
          </div>
        </CollapsibleContent>
      </Collapsible>
    </div>
  {/if}
</div>

<style>
  .prose :global(h1),
  .prose :global(h2),
  .prose :global(h3),
  .prose :global(h4) {
    color: #363636;
    font-weight: 700;
    margin-top: 1.2em;
    margin-bottom: 0.4em;
  }
  .prose :global(h1) { font-size: 1.25rem; }
  .prose :global(h2) { font-size: 1.1rem; }
  .prose :global(h3) { font-size: 1rem; }
  .prose :global(p) {
    color: #4a5568;
    font-size: 0.875rem;
    line-height: 1.65;
    margin-bottom: 0.75em;
  }
  .prose :global(ul),
  .prose :global(ol) {
    padding-left: 1.4em;
    margin-bottom: 0.75em;
    font-size: 0.875rem;
    color: #4a5568;
    line-height: 1.65;
  }
  .prose :global(ul) { list-style-type: disc; }
  .prose :global(ol) { list-style-type: decimal; }
  .prose :global(li) { margin-bottom: 0.25em; }
  .prose :global(code) {
    background: #f4f8fe;
    color: #36547f;
    border-radius: 3px;
    padding: 0.1em 0.35em;
    font-size: 0.82rem;
  }
  .prose :global(pre) {
    background: #f4f8fe;
    border-radius: 6px;
    padding: 1em;
    overflow-x: auto;
    margin-bottom: 0.75em;
  }
  .prose :global(pre code) {
    background: none;
    padding: 0;
    font-size: 0.82rem;
    color: #363636;
  }
  .prose :global(a) {
    color: #4a90e2;
    text-decoration: underline;
  }
  .prose :global(blockquote) {
    border-left: 3px solid #8fb4ec;
    margin-left: 0;
    padding-left: 1em;
    color: #8fb4ec;
    font-style: italic;
  }
  .prose :global(hr) {
    border: none;
    border-top: 1px solid #f4f8fe;
    margin: 1em 0;
  }
</style>
