<script lang="ts">
  import type { ProjectPart, ProjectPartState } from "$lib/api";

  let {
    parts,
    currentId,
    campaignSlug,
    campaignName,
  }: {
    /** The campaign's parts in play order, including this one. */
    parts: ProjectPart[];
    currentId: string;
    campaignSlug: string | null;
    campaignName: string;
  } = $props();

  const index = $derived(parts.findIndex((p) => p.id === currentId));
  const previous = $derived(index > 0 ? parts[index - 1] : undefined);
  const next = $derived(index >= 0 && index < parts.length - 1 ? parts[index + 1] : undefined);

  const STATE_NOTE: Record<ProjectPartState, string> = {
    completed: "Completed",
    in_progress: "In progress",
    available: "Ready to play",
    locked: "Unlocks when you finish this part",
  };

  function href(part: ProjectPart): string {
    return `/projects/${part.slug ?? part.id}`;
  }
</script>

<!-- Where this part sits in its campaign. A part page is an ordinary project
     page — its own tasks, sessions and board — with this one strip on top so
     the player can step back and forth without going through the campaign. -->
{#if index >= 0}
  <nav
    class="mt-[24px] flex flex-col gap-[12px] rounded-[8px] bg-white px-[20px] py-[16px] sm:flex-row sm:items-center sm:justify-between"
    aria-label="Campaign navigation"
  >
    <div class="min-w-0">
      <p class="text-[12px] font-semibold uppercase tracking-wide text-brand-muted">
        Part {index + 1} of {parts.length}
      </p>
      <a
        href={campaignSlug ? `/projects/${campaignSlug}` : "/projects"}
        class="font-heading text-[18px] font-bold text-brand-text hover:text-brand-blue"
      >
        {campaignName}
      </a>
    </div>

    <div class="flex flex-wrap items-center gap-[8px]">
      {#if previous}
        <a
          href={href(previous)}
          class="flex min-w-0 max-w-[240px] flex-col rounded-[8px] bg-brand-light-blue px-[12px] py-[8px] transition-opacity hover:opacity-80"
        >
          <span class="text-[11px] font-semibold uppercase tracking-wide text-brand-muted">
            ← Previous
          </span>
          <span class="truncate text-[14px] font-semibold text-brand-text">{previous.name}</span>
        </a>
      {/if}

      {#if next}
        <a
          href={href(next)}
          class="flex min-w-0 max-w-[240px] flex-col rounded-[8px] bg-brand-light-blue px-[12px] py-[8px] text-right transition-opacity hover:opacity-80"
          title={STATE_NOTE[next.state]}
        >
          <span class="text-[11px] font-semibold uppercase tracking-wide text-brand-muted">
            Next {next.state === "locked" ? "· locked" : ""} →
          </span>
          <span class="truncate text-[14px] font-semibold text-brand-text">{next.name}</span>
        </a>
      {:else}
        <span
          class="rounded-[8px] bg-brand-light-blue px-[12px] py-[8px] text-[13px] font-semibold text-brand-muted"
        >
          Last part of the campaign
        </span>
      {/if}
    </div>
  </nav>
{/if}
