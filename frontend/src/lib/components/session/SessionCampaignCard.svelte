<script lang="ts">
  import type { SessionCampaign } from "$lib/api";
  import { formatDateUTC } from "$lib/format";

  let { campaign }: { campaign: SessionCampaign } = $props();

  const total = $derived(campaign.parts.length);
  const currentIndex = $derived(campaign.parts.findIndex((p) => p.current));
  const over = $derived(
    campaign.session_status === "finished" || campaign.session_status === "cancelled",
  );

  // What the row for *this* session's part says. "Playing now" is a claim
  // about the present tense, so a session that has ended reports what came of
  // it instead: cleared here, or ended without clearing it.
  function currentNote(cleared: boolean): string {
    if (!over) return "Playing now";
    if (cleared) return "Cleared here";
    return campaign.session_status === "cancelled" ? "Session cancelled" : "Not cleared";
  }

  function partHref(slug: string | null, id: string): string {
    return `/projects/${slug ?? id}`;
  }

  function when(iso: string | null): string {
    return iso ? formatDateUTC(iso) : "";
  }
</script>

<!--
  Where this session sits in its campaign, and what its players brought with
  them. A part continues the codebase of the part before it, so a dashboard
  that says only "Part 3" hides the half of the story that explains the code:
  who already cleared parts 1 and 2, and in which session that work can be
  read. Parts still ahead are listed greyed, so the arc has a visible end.
-->
<section
  class="mb-[20px] rounded-[8px] bg-white px-[24px] py-[20px]"
  aria-label="Campaign progress"
  data-testid="session-campaign"
>
  <div class="mb-[14px] flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
    <h2 class="font-heading text-[16px] font-bold" style="color: #363636;">
      {#if campaign.slug}
        <a href="/projects/{campaign.slug}" class="hover:text-brand-blue">{campaign.name}</a>
      {:else}
        {campaign.name}
      {/if}
    </h2>
    <span class="text-[13px] text-brand-muted">
      Part {campaign.current_part_ordinal + 1} of {total}
    </span>
  </div>

  <ol class="flex flex-col">
    {#each campaign.parts as part, i (part.project_id)}
      {@const ahead = i > currentIndex}
      <li
        class="flex flex-col gap-[4px] py-[10px] sm:flex-row sm:items-baseline sm:gap-[12px]"
        class:border-t={i > 0}
        style="border-color: #eef3fb;"
        data-testid="campaign-part-{part.part_ordinal}"
      >
        <span
          class="flex h-[24px] w-[24px] shrink-0 items-center justify-center rounded-full text-[12px] font-bold tabular-nums
            {part.current
            ? 'bg-brand-blue text-white'
            : part.cleared_by.length > 0
              ? 'bg-emerald-50 text-emerald-700'
              : 'bg-[#e2ecfc] text-[#8fb4ec]'}"
        >
          {part.part_ordinal + 1}
        </span>

        <div class="min-w-0 flex-1">
          <a
            href={partHref(part.slug, part.project_id)}
            class="text-[14px] font-semibold {ahead
              ? 'text-brand-muted'
              : 'text-brand-text'} hover:text-brand-blue"
          >
            {part.name}
          </a>
          {#if part.cleared_by.length > 0}
            <div class="mt-[2px] flex flex-wrap items-center gap-x-2 gap-y-1 text-[13px]">
              {#each part.cleared_by as who (who.user_id)}
                {#if part.current}
                  <!-- Cleared in this very session: a link here would go
                       nowhere the reader is not already. -->
                  <span
                    class="inline-flex items-center gap-1 rounded-full bg-emerald-50 px-2 py-0.5 font-medium text-emerald-700"
                    data-testid="campaign-cleared-{part.part_ordinal}-{who.user_id}"
                  >
                    <span aria-hidden="true">✓</span>
                    {who.display_name}
                  </span>
                {:else}
                  <a
                    href="/s/{who.join_code}"
                    class="inline-flex items-center gap-1 rounded-full bg-emerald-50 px-2 py-0.5 font-medium text-emerald-700 hover:underline"
                    title="Cleared in session {who.join_code}{when(who.finished_at)
                      ? ` · ${when(who.finished_at)}`
                      : ''}"
                    data-testid="campaign-cleared-{part.part_ordinal}-{who.user_id}"
                  >
                    <span aria-hidden="true">✓</span>
                    {who.display_name}
                  </a>
                {/if}
              {/each}
            </div>
          {/if}
        </div>

        <span class="shrink-0 text-[12px] font-semibold uppercase tracking-wide text-brand-muted">
          {#if part.current}
            {currentNote(part.cleared_by.length > 0)}
          {:else if part.cleared_by.length > 0}
            Done
          {:else if ahead}
            Ahead
          {:else}
            Not cleared
          {/if}
        </span>
      </li>
    {/each}
  </ol>
</section>
