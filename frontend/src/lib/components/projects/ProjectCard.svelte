<script lang="ts">
  import { ikCover } from "$lib/imagekit";
  import type { Project, ProjectPartState } from "$lib/api";

  let {
    project,
    eager = false,
    onStart,
    status = null,
  }: {
    project: Project;
    /** First above-the-fold card: eager-load its cover for LCP. */
    eager?: boolean;
    /** When set, the card offers "Start session"; omit to hide the button. */
    onStart?: (slug: string) => void;
    /** Progression badge for a campaign part — the one place this card is
     *  used for something a player unlocks rather than simply opens. */
    status?: { label: string; tone: ProjectPartState } | null;
  } = $props();

  // Tones live here rather than in the caller so the card stays the single
  // owner of its own look.
  const STATUS_CLASS: Record<ProjectPartState, string> = {
    completed: "bg-emerald-50 text-emerald-700",
    in_progress: "bg-amber-50 text-amber-700",
    available: "bg-white/90 text-[#0269fb]",
    locked: "bg-white/90 text-[#6b7a90]",
  };

  // Descriptions are markdown; the card teaser wants plain prose. Strip the
  // syntax that reads as noise in two clamped lines (emphasis, code ticks,
  // links, headings, list bullets) without pulling in a parser.
  function teaser(md: string): string {
    return md
      .replace(/```[\s\S]*?```/g, " ")
      // Table rows read as pipe soup in two clamped lines.
      .replace(/^\s*\|.*$/gm, " ")
      .replace(/`([^`]*)`/g, "$1")
      .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
      .replace(/^#{1,6}\s+/gm, "")
      .replace(/^\s*[-*]\s+/gm, "")
      .replace(/\*\*?([^*]+)\*\*?/g, "$1")
      .replace(/\s+/g, " ")
      .trim();
  }

  // A campaign is played part by part, so its card answers "how long is all
  // of this" rather than repeating a session length it never runs.
  const duration = $derived(project.parts_duration_secs ?? project.session_duration_secs);
  const durationTitle = $derived(
    project.parts_duration_secs
      ? `Total playing time across ${project.part_count} parts`
      : "How long one session runs",
  );

  // Category · size · reviews · played, as segments so a missing one cannot
  // leave a stray separator behind. A campaign has no tasks of its own —
  // "0 tasks" read as an empty project — and the cover badge already counts
  // its parts, so its size segment is simply dropped.
  const metaBits = $derived.by(() => {
    const bits: { text: string; title?: string }[] = [];
    if (project.category) bits.push({ text: project.category });
    if ((project.part_count ?? 0) === 0) {
      bits.push({
        text: `${project.task_count} ${project.task_count === 1 ? 'task' : 'tasks'}`,
      });
    }
    if ((project.judge_review_count ?? 0) > 0) {
      bits.push({
        text: `~${project.judge_review_count} ${project.judge_review_count === 1 ? 'review' : 'reviews'}`,
        title:
          'Estimated judge reviews a full session triggers (counts toward your monthly quota)',
      });
    }
    if ((project.session_count ?? 0) > 0) {
      bits.push({ text: `${project.session_count} played` });
    }
    return bits;
  });

  function formatDuration(secs: number): string {
    const mins = Math.round(secs / 60);
    if (mins < 60) return `${mins} min`;
    const h = Math.floor(mins / 60);
    const rest = mins % 60;
    return rest === 0 ? `${h} h` : `${h} h ${rest} min`;
  }
</script>

<!-- The catalog card (audit UI-M3), shared by /projects and the landing so
     the two grids cannot drift apart: cover with a light brand tint,
     category · tasks · played meta, a clamped teaser, tag chips and an
     Open / Start session footer. -->
<a
  href="/projects/{project.slug ?? project.id}"
  class="flex flex-col overflow-hidden rounded-[8px] bg-white transition-shadow hover:shadow-[0_6px_32px_0_rgba(19,101,218,0.16)]
    {project.archived_at !== null ? 'opacity-70 grayscale' : ''} {status?.tone === 'locked'
    ? 'opacity-75'
    : ''}"
>
  <!-- card image -->
  <span class="relative block h-[190px] overflow-hidden rounded-t-[8px]">
    <!-- Brand tint kept light: covers often carry baked-in text that a
         heavy overlay used to drown (audit UI-M3). -->
    <span
      class="absolute inset-0 z-10 rounded-t-[8px] opacity-[0.38]"
      style="background-image: linear-gradient(55deg, #1677ff, #96c2ff);"
    ></span>
    <img
      src={project.cover_image_url ? ikCover(project.cover_image_url, 760, 380) : '/item-img.webp'}
      alt={project.name}
      class="block h-full w-full object-cover object-center"
      loading={eager ? 'eager' : 'lazy'}
      fetchpriority={eager ? 'high' : 'auto'}
      decoding="async"
    />
    {#if project.archived_at !== null}
      <span
        class="absolute left-2 top-2 z-20 flex items-center gap-1 rounded bg-black/60 px-2 py-0.5 text-[11px] font-semibold text-white"
      >
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="11"
          height="11"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2.5"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <polyline points="21 8 21 21 3 21 3 8" />
          <rect x="1" y="3" width="22" height="5" />
          <line x1="10" y1="12" x2="14" y2="12" />
        </svg>
        Archived
      </span>
    {/if}
    <span
      class="absolute right-2 top-2 z-20 flex items-center gap-1 rounded bg-white/90 px-2 py-0.5 text-[11px] font-semibold text-[#0269fb]"
      title={durationTitle}
    >
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width="11"
        height="11"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2.5"
        stroke-linecap="round"
        stroke-linejoin="round"
      >
        <circle cx="12" cy="12" r="10" />
        <polyline points="12 6 12 12 16 14" />
      </svg>
      {formatDuration(duration)}
    </span>
    <!-- A campaign part carries its progression where the campaign badge
         would sit; the two never appear on the same card. -->
    {#if status}
      <span
        class="absolute left-2 top-2 z-20 rounded px-2 py-0.5 text-[11px] font-semibold {STATUS_CLASS[
          status.tone
        ]}"
        data-testid="project-part-state"
      >
        {status.label}
      </span>
    {/if}
    <!-- A campaign is several sessions long; say so on the card, because the
         duration chip above only covers one part. -->
    {#if (project.part_count ?? 0) > 0}
      <span
        class="absolute left-2 top-2 z-20 flex items-center gap-1 rounded bg-white/90 px-2 py-0.5 text-[11px] font-semibold text-[#0269fb]"
        title="A campaign played one part at a time"
        data-testid="project-campaign-parts"
      >
        {project.part_count} parts
      </span>
    {/if}
    <!-- Per-user "you've played this" badge: only for a signed-in caller who
         has finished at least one session here (audit: personal play history). -->
    {#if (project.user_session_count ?? 0) > 0}
      <span
        class="absolute bottom-2 left-2 z-20 flex items-center gap-1 rounded bg-white/90 px-2 py-0.5 text-[11px] font-semibold text-[#16a34a]"
        title="You've played this project"
        data-testid="project-user-played"
      >
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="11"
          height="11"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="3"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <polyline points="20 6 9 17 4 12" />
        </svg>
        Played{#if (project.user_session_count ?? 0) > 1}
          {project.user_session_count}×{/if}
      </span>
    {/if}
  </span>
  <!-- card body -->
  <span class="flex flex-grow flex-col rounded-b-[8px] bg-white p-[24px]">
    <span class="mb-[6px] block font-heading text-[18px] font-bold leading-[1.33] text-[#363636]">
      {project.name}
    </span>

    <!-- meta row: category · task count · played counter -->
    <span class="mb-[8px] flex flex-wrap items-center gap-x-[8px] gap-y-[2px] text-[12px] font-semibold text-[#3061ac]">
      {#each metaBits as bit, i (bit.text)}
        {#if i > 0}
          <span class="text-[#b8d4f8]">·</span>
        {/if}
        <span title={bit.title}>{bit.text}</span>
      {/each}
    </span>

    {#if project.description}
      <!-- no `block` here: line-clamp needs its own -webkit-box display -->
      <span class="mb-[10px] line-clamp-2 text-[13px] leading-[1.55] text-[#6b7a90]">
        {teaser(project.description)}
      </span>
    {/if}

    {#if project.tags.length > 0}
      <span class="mb-[10px] flex flex-wrap gap-[4px]">
        {#each project.tags.slice(0, 3) as tag (tag)}
          <span class="rounded-[8px] bg-[#dce9fc] px-[8px] text-[11px] font-semibold leading-[1.8] text-[#3061ac]">
            {tag}
          </span>
        {/each}
        {#if project.tags.length > 3}
          <span class="px-[2px] text-[11px] font-semibold leading-[1.8] text-[#9ea7b6]">
            +{project.tags.length - 3}
          </span>
        {/if}
      </span>
    {/if}

    <!-- footer: open + start session -->
    <span class="mt-auto flex items-center justify-between pt-[4px]">
      <span class="flex items-center text-base font-bold leading-[1.6] text-[#0269fb]">
        Open
        <svg
          class="ml-1 fill-[#0269fb]"
          xmlns="http://www.w3.org/2000/svg"
          width="11"
          height="12"
          viewBox="0 0 306 306"
        >
          <path d="M94.35 0l-35.7 35.7L175.95 153 58.65 270.3l35.7 35.7 153-153z" />
        </svg>
      </span>
      <!-- A campaign hosts no sessions — `ololo start <campaign>` is refused
           by the server — so its card opens the campaign and lets the player
           pick a part. Same for a locked part: nothing to start yet. -->
      {#if onStart && project.slug && project.archived_at === null && (project.part_count ?? 0) === 0 && status?.tone !== "locked"}
        <button
          type="button"
          onclick={(e) => {
            e.preventDefault();
            e.stopPropagation();
            onStart(project.slug ?? "");
          }}
          class="rounded-[6px] border border-[#0269fb] px-[12px] py-[4px] text-[13px] font-semibold text-[#0269fb] transition-colors hover:bg-[#0269fb] hover:text-white"
        >
          Start session
        </button>
      {/if}
    </span>
  </span>
</a>
