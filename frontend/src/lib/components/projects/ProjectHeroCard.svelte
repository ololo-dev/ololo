<script lang="ts">
  import { ikWidth } from '$lib/imagekit';
  import type { Project, ProjectJudge, ProjectPart } from "$lib/api";
  import { formatDuration } from "$lib/format";
  import MarkdownContent from "$lib/components/MarkdownContent.svelte";
  import JudgeChips from "$lib/components/projects/JudgeChips.svelte";

  let {
    project,
    judges = [],
    currentUserId,
    isAdmin,
    sessionCount,
    hasActiveSessions,
    onStart,
    onStartPart,
    nextPart = null,
    lockedReason = null,
  }: {
    project: Project;
    judges?: ProjectJudge[];
    currentUserId: string | null | undefined;
    isAdmin: boolean;
    sessionCount: number;
    hasActiveSessions: boolean;
    onStart: () => void;
    /** Start a specific part of this campaign, by slug. */
    onStartPart?: (slug: string) => void;
    /** The part a campaign wants the viewer to play next; null when the
     *  ladder is finished, or when this is not a campaign. */
    nextPart?: ProjectPart | null;
    /** Why this campaign part cannot be started yet; absent when it can. */
    lockedReason?: string | null;
  } = $props();

  // A campaign hosts no sessions of its own — its parts do. Counting them
  // here could only ever print "Sessions 0 · Active session No", which reads
  // as a dead project rather than as one you play a part at a time.
  const isCampaign = $derived((project.part_count ?? 0) > 0);

</script>

<!-- 2-col card -->
<div class="flex flex-col rounded-[8px] bg-white lg:flex-row">
  <!-- Left: hero with cover image or gradient -->
  <div
    class="relative flex flex-1 flex-col justify-between overflow-hidden rounded-t-[8px] p-6 text-white sm:p-[48px] lg:rounded-l-[8px] lg:rounded-tr-none"
    style={project.cover_image_url
      ? `background-image: url('${ikWidth(project.cover_image_url, 1600)}'); background-size: cover; background-position: center;`
      : "background-image: linear-gradient(57deg, #1677ff, #96c2ff);"}
  >
    <!-- Dimming overlay -->
    <div
      class="pointer-events-none absolute inset-0 rounded-l-[8px] opacity-[0.93]"
      style="background-image: linear-gradient(57deg, #1677ff, #96c2ff);"
    ></div>

    <!-- Edit stays contextual; everything with a blast radius (archive,
         re-read, export, delete) lives in Settings → Projects, where the
         actions sit in a table with room and a confirm — not as five
         translucent icons over a photo with the trash one slot from Export
         (audit UI-H3). -->
    {#if currentUserId === project.owner_user_id || isAdmin}
      <div class="absolute right-[16px] top-[16px] z-[3] flex items-center gap-[8px]">
        {#if isAdmin}
          <a
            href="/settings/projects"
            data-testid="manage-projects-link"
            title="Archive, export, sync or delete — in the projects console"
            class="flex h-[32px] items-center rounded-full bg-white/20 px-3 text-xs font-semibold text-white transition-opacity hover:bg-white/35"
          >
            Manage
          </a>
        {/if}
        {#if !project.archived_at}
          <a
            href="/projects/{project.id}/edit"
            data-testid="edit-project-btn"
            title="Edit project"
            class="flex h-[32px] w-[32px] items-center justify-center rounded-full bg-white/20 text-white transition-opacity hover:bg-white/35"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="16"
              height="16"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path
                d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"
              />
              <path
                d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"
              />
            </svg>
          </a>
        {/if}
      </div>
    {/if}

    <!-- Top content. The title clears the floating Manage/Edit controls in
         the corner — without the reserved padding a long name runs under
         them on narrow screens. -->
    <div class="relative z-[2] w-full">
      <h2
        class="mb-[16px] font-heading text-[28px] font-bold leading-tight sm:text-[34px] {currentUserId ===
          project.owner_user_id || isAdmin
          ? 'pr-[88px]'
          : ''}"
      >
        {project.name}
      </h2>

      <!-- Badge: visibility + archived (category and tags live in the sidebar) -->
      <div class="mb-[28px] flex flex-wrap gap-2">
        {#if project.parent_project_slug && project.part_ordinal != null}
          <a
            href="/projects/{project.parent_project_slug}"
            class="rounded-[4px] bg-white/25 px-[12px] py-[4px] text-[14px] font-semibold transition-opacity hover:opacity-80"
          >
            Part {project.part_ordinal + 1} of {project.parent_project_name ?? "the campaign"}
          </a>
        {/if}
        {#if (project.part_count ?? 0) > 0}
          <span class="rounded-[4px] bg-white/20 px-[12px] py-[4px] text-[14px] font-semibold">
            Campaign · {project.part_count} parts
          </span>
        {/if}
        <span
          class="rounded-[4px] bg-white/20 px-[12px] py-[4px] text-[14px] font-semibold"
        >
          {project.public ? "Public" : "Private"}
        </span>
        {#if project.archived_at}
          <span
            class="rounded-[4px] bg-white/20 px-[12px] py-[4px] text-[14px] font-semibold"
          >
            Archived
          </span>
        {/if}
      </div>

      {#if project.description}
        <div class="text-sm leading-relaxed">
          <MarkdownContent
            value={project.description}
            class="prose-invert"
            style="--tw-prose-body: white; --tw-prose-lead: rgba(255,255,255,0.9); --tw-prose-bold: white; --tw-prose-headings: white; --tw-prose-bullets: rgba(255,255,255,0.75); --tw-prose-counters: rgba(255,255,255,0.75);"
          />
        </div>
      {/if}

      {#if judges.length > 0}
        <div class="mt-[28px]">
          <div
            class="mb-[10px] flex items-center gap-[6px] text-[12px] font-semibold uppercase tracking-wide text-white/70"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path d="m14 13-7.5 7.5a2.12 2.12 0 0 1-3-3L11 10" />
              <path d="m16 16 6-6" />
              <path d="m8 8 6-6" />
              <path d="m9 7 8 8" />
              <path d="m21 11-8-8" />
            </svg>
            Judges ({judges.length})
          </div>
          <JudgeChips {judges} />
        </div>
      {/if}

    </div>
  </div>

  <!-- Right: stats bar -->
  <div
    class="flex w-full flex-shrink-0 flex-col justify-between px-6 py-6 lg:w-[300px] lg:px-[48px] lg:py-[32px]"
  >
    <!-- Fields -->
    <div>
      {#if isCampaign}
        <div class="mb-[16px] leading-[1.5]">
          <div class="text-[12px] font-semibold leading-[1.33] text-brand-muted">
            Parts
          </div>
          <p class="text-brand-text">{project.part_count}</p>
        </div>
      {:else}
        <div class="mb-[16px] leading-[1.5]">
          <div class="text-[12px] font-semibold leading-[1.33] text-brand-muted">
            Sessions
          </div>
          <p class="text-brand-text">{sessionCount}</p>
        </div>
      {/if}
      <div class="mb-[16px] leading-[1.5]">
        <div class="text-[12px] font-semibold leading-[1.33] text-brand-muted">
          Visibility
        </div>
        <p class="text-brand-text">
          {project.public ? "Public" : "Private"}
        </p>
      </div>
      {#if project.category}
        <div class="mb-[16px] leading-[1.5]">
          <div class="text-[12px] font-semibold leading-[1.33] text-brand-muted">
            Category
          </div>
          <p class="text-brand-text">{project.category}</p>
        </div>
      {/if}
      {#if project.slug}
        <div class="mb-[16px] leading-[1.5]">
          <div class="text-[12px] font-semibold leading-[1.33] text-brand-muted">
            Slug
          </div>
          <p class="truncate font-mono text-sm text-brand-text">
            {project.slug}
          </p>
        </div>
      {/if}
      <div class="mb-[16px] leading-[1.5]">
        <div class="text-[12px] font-semibold leading-[1.33] text-brand-muted">
          {project.parts_duration_secs ? "Total playing time" : "Duration"}
        </div>
        <p class="text-brand-text">
          {formatDuration(project.parts_duration_secs ?? project.session_duration_secs)}
          {#if project.parts_duration_secs}
            <span class="text-brand-muted">across {project.part_count} parts</span>
          {/if}
        </p>
      </div>
      {#if (project.judge_review_count ?? 0) > 0}
        <div class="mb-[16px] leading-[1.5]">
          <div
            class="text-[12px] font-semibold leading-[1.33] text-brand-muted"
            title="Judges run once per task per player, and each counts toward your monthly judge-run quota"
          >
            Judge reviews
          </div>
          <p class="text-brand-text">~{project.judge_review_count} per session</p>
        </div>
      {/if}
      {#if !isCampaign}
        <div class="mb-[16px] leading-[1.5]">
          <div class="text-[12px] font-semibold leading-[1.33] text-brand-muted">
            Active session
          </div>
          <p class="text-brand-text">{hasActiveSessions ? "Yes" : "No"}</p>
        </div>
      {/if}
      {#if project.points_range}
        <div class="mb-[16px] leading-[1.5]">
          <div class="text-[12px] font-semibold leading-[1.33] text-brand-muted">
            Points
          </div>
          <p class="text-brand-text">
            {project.points_range.min === project.points_range.max
              ? `${project.points_range.min}`
              : `${project.points_range.min}\u2013${project.points_range.max}`}
          </p>
        </div>
      {/if}
      {#if project.tags.length > 0}
        <div class="mb-[16px] leading-[1.5]">
          <div class="mb-[6px] text-[12px] font-semibold leading-[1.33] text-brand-muted">
            Tags
          </div>
          <ul class="flex flex-wrap gap-[6px]">
            {#each project.tags as tag}
              <li
                class="rounded-full bg-brand-light-blue px-[10px] py-[3px] text-[13px] font-medium text-brand-blue"
              >
                {tag}
              </li>
            {/each}
          </ul>
        </div>
      {/if}
    </div>

    <!-- Footer: start session button (only when slug is set).
         A campaign parent hosts no sessions — its parts do — and a locked
         part says what unlocks it instead of failing at the CLI. -->
    {#if (project.part_count ?? 0) > 0}
      <!-- A campaign hosts no sessions of its own, but it knows which of its
           parts the viewer is due to play — so the page offers that one
           instead of asking them to go and find it. -->
      {#if nextPart && nextPart.state === "available" && nextPart.slug}
        <div class="mt-[24px]">
          <button
            type="button"
            onclick={() => onStartPart?.(nextPart.slug ?? "")}
            data-testid="campaign-start-next"
            class="block w-full rounded-[8px] bg-brand-blue px-4 py-[12px] text-center text-[16px] font-semibold text-white transition-opacity hover:opacity-80"
          >
            Start part {(nextPart.part_ordinal ?? 0) + 1}
          </button>
          <p class="mt-[8px] text-center text-[13px] text-brand-muted">{nextPart.name}</p>
        </div>
      {:else if nextPart && nextPart.state === "in_progress" && nextPart.slug}
        <div class="mt-[24px]">
          <a
            href="/projects/{nextPart.slug}"
            data-testid="campaign-continue"
            class="block w-full rounded-[8px] bg-brand-blue px-4 py-[12px] text-center text-[16px] font-semibold text-white transition-opacity hover:opacity-80"
          >
            Continue part {(nextPart.part_ordinal ?? 0) + 1}
          </a>
          <p class="mt-[8px] text-center text-[13px] text-brand-muted">{nextPart.name}</p>
        </div>
      {:else if nextPart === null && (project.part_count ?? 0) > 0}
        <div class="mt-[24px] rounded-[8px] bg-emerald-50 px-4 py-[12px] text-center text-[15px] font-semibold text-emerald-700">
          Campaign complete — every part cleared.
        </div>
      {:else}
        <div class="mt-[24px] rounded-[8px] bg-brand-light-blue px-4 py-[12px] text-center text-[15px] text-brand-text">
          Pick a part below to start playing.
        </div>
      {/if}
    {:else if project.slug && lockedReason}
      <!-- No button at all: a locked part offers nothing to press, and a
           greyed-out one only invites the click it is about to refuse. -->
      <div
        class="mt-[24px] rounded-[8px] bg-brand-light-blue px-4 py-[12px] text-center text-[15px] text-brand-text"
        data-testid="project-locked-note"
      >
        {lockedReason}
      </div>
    {:else if project.slug}
      <div class="mt-[24px]">
        <button
          type="button"
          onclick={onStart}
          class="block w-full rounded-[8px] bg-brand-blue px-4 py-[12px] text-center text-[16px] font-semibold text-white transition-opacity hover:opacity-80"
        >
          Start session
        </button>
      </div>
    {/if}
  </div>
</div>