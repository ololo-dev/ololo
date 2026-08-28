<script lang="ts">
  import ProjectCard from "$lib/components/projects/ProjectCard.svelte";
  import type { ProjectPart, ProjectPartState } from "$lib/api";

  let {
    parts,
    signedIn = false,
    onStart,
  }: {
    parts: ProjectPart[];
    signedIn?: boolean;
    /** Offered on the parts a player can actually start right now. */
    onStart?: (slug: string) => void;
  } = $props();

  // A part is startable when the campaign says it is open and nothing of the
  // player's is already running in it. Locked parts are refused by the server
  // on session creation, and a part already in progress is joined from its own
  // page rather than started a second time.
  function startable(part: ProjectPart): boolean {
    return !!onStart && !!part.slug && (part.state === "available" || part.state === "completed");
  }

  // Cards lead: a part is a project, so browsing a campaign should feel like
  // browsing the catalog — same card, same everything. The list is the
  // compact read for a long campaign or a narrow screen.
  type View = "cards" | "list";
  let view = $state<View>("cards");

  const STATE_LABEL: Record<ProjectPartState, string> = {
    completed: "Completed",
    in_progress: "In progress",
    available: "Ready to play",
    locked: "Locked",
  };

  const STATE_CLASS: Record<ProjectPartState, string> = {
    completed: "bg-emerald-50 text-emerald-700",
    in_progress: "bg-amber-50 text-amber-700",
    available: "bg-brand-light-blue text-brand-blue",
    locked: "bg-[#f1f3f7] text-brand-muted",
  };

  // A locked part names what unlocks it, so the ladder reads as a route
  // rather than a wall.
  function lockedBecause(index: number): string {
    const previous = parts[index - 1];
    return previous ? `Finish “${previous.name}” first` : "Locked";
  }

  function meta(part: ProjectPart, index: number): string {
    const base = `${part.task_count} tasks · ${Math.max(1, Math.round(part.session_duration_secs / 60))} min`;
    return part.state === "locked" ? `${base} · ${lockedBecause(index)}` : base;
  }
</script>

<!-- The campaign ladder. Locked parts stay visible and clickable — seeing
     what comes next is the point of a campaign — the gate lives on session
     creation, not on the link. -->
{#if parts.length === 0}
  <p class="mt-[16px] text-brand-muted">This campaign has no parts yet.</p>
{:else}
  <div class="mt-[16px] flex flex-wrap items-center justify-between gap-[12px]">
    {#if !signedIn}
      <p class="text-[13px] text-brand-muted">
        Sign in to see how far you have got — parts unlock one after another as you finish them.
      </p>
    {:else}
      <span></span>
    {/if}

    <div
      class="flex shrink-0 items-center gap-[2px] rounded-[8px] bg-white p-[3px]"
      role="group"
      aria-label="Part layout"
    >
      {#each [{ key: "cards", label: "Cards" }, { key: "list", label: "List" }] as option (option.key)}
        <button
          type="button"
          aria-pressed={view === option.key}
          onclick={() => (view = option.key as View)}
          class="rounded-[6px] px-[12px] py-[5px] text-[13px] font-semibold transition-colors
            {view === option.key
            ? 'bg-brand-light-blue text-brand-blue'
            : 'text-brand-muted hover:text-brand-text'}"
        >
          {option.label}
        </button>
      {/each}
    </div>
  </div>

  {#if view === "cards"}
    <!-- The catalog's own card, so a part looks exactly like the project it
         is — and the one you can play carries the same Start button the
         catalog does, because walking into the part's page to find it was a
         step that answered nothing. -->
    <div class="mt-[16px] grid grid-cols-1 gap-[30px] md:grid-cols-2 lg:grid-cols-3">
      {#each parts as part (part.id)}
        <ProjectCard
          project={part}
          status={{ label: STATE_LABEL[part.state], tone: part.state }}
          onStart={startable(part) ? onStart : undefined}
        />
      {/each}
    </div>
  {:else}
    <ol class="mt-[16px] flex flex-col gap-[8px]">
      {#each parts as part, i (part.id)}
        <li>
          <a
            href="/projects/{part.slug ?? part.id}"
            class="flex flex-wrap items-center gap-x-[12px] gap-y-[6px] rounded-[8px] bg-white px-[16px] py-[14px] transition-colors hover:bg-[#f9fbff] sm:flex-nowrap sm:gap-[16px] sm:px-[24px] sm:py-[16px]"
            class:opacity-70={part.state === "locked"}
          >
            <span
              class="flex h-[32px] w-[32px] shrink-0 items-center justify-center rounded-full text-[14px] font-bold tabular-nums"
              style="background: #e2ecfc; color: #8fb4ec;"
            >
              {i + 1}
            </span>

            <div class="min-w-0 flex-1">
              <p class="truncate font-semibold text-brand-text">{part.name}</p>
              <p class="hidden text-[13px] text-brand-muted sm:block">{meta(part, i)}</p>
            </div>

            <span
              class="shrink-0 rounded-full px-[10px] py-[4px] text-[12px] font-semibold {STATE_CLASS[
                part.state
              ]}"
            >
              {STATE_LABEL[part.state]}
            </span>

            {#if startable(part)}
              <button
                type="button"
                onclick={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  onStart?.(part.slug ?? "");
                }}
                data-testid="campaign-start-{part.part_ordinal}"
                class="shrink-0 rounded-[6px] border border-brand-blue px-[12px] py-[4px] text-[13px] font-semibold text-brand-blue transition-colors hover:bg-brand-blue hover:text-white"
              >
                Start session
              </button>
            {/if}

            <p class="w-full pl-[44px] text-[12px] leading-snug text-brand-muted sm:hidden">
              {meta(part, i)}
            </p>
          </a>
        </li>
      {/each}
    </ol>
  {/if}
{/if}
