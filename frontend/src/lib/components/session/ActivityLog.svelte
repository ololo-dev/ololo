<script lang="ts">
  import { ikAvatar } from '$lib/imagekit';
  import ImageLightbox from '$lib/components/sessions/ImageLightbox.svelte';
  import ClampedText from './ClampedText.svelte';
  import MarkdownContent from '$lib/components/MarkdownContent.svelte';
  import type { ActivityEvent } from "$lib/types/arena";

  let {
    events,
    startedAt = null,
    avatarByPlayerId = new Map<string, string | null>(),
    sessionId = null,
  }: {
    events: ActivityEvent[];
    startedAt?: Date | null;
    avatarByPlayerId?: Map<string, string | null>;
    /** Needed to build artifact URLs; without it artifacts show as text. */
    sessionId?: string | null;
  } = $props();

  function artifactUrl(ev: ActivityEvent, i = 0): string | null {
    const probeId = ev.detail?.probe_id;
    if (!sessionId || !probeId) return null;
    const base = `/api/sessions/${sessionId}/artifacts/${probeId}`;
    return i > 0 ? `${base}?i=${i}` : base;
  }

  /** Every delivered file; older entries carried only the first one. */
  function artifactFiles(ev: ActivityEvent): { path: string; size: number }[] {
    const files = ev.detail?.files;
    if (files?.length) return files;
    const path = ev.detail?.path;
    return path ? [{ path, size: ev.detail?.size ?? 0 }] : [];
  }

  function fileName(path: string): string {
    return path.split("/").pop() || "artifact";
  }

  function artifactFileName(ev: ActivityEvent): string {
    return fileName(ev.detail?.path ?? "");
  }

  function sizeLabel(size: number): string {
    if (size >= 1024 * 1024) return `${(size / (1024 * 1024)).toFixed(1)} MB`;
    if (size >= 1024) return `${Math.round(size / 1024)} KB`;
    return `${size} B`;
  }

  function artifactSizeLabel(ev: ActivityEvent): string {
    return sizeLabel(artifactFiles(ev).reduce((s, f) => s + f.size, 0));
  }

  function totalSize(ev: ActivityEvent): number {
    return artifactFiles(ev).reduce((s, f) => s + f.size, 0);
  }

  // ---- Filters: by player (only offered with several players) and by
  // record type. Chips, not dropdowns — one click, state visible.
  type TypeFilter = "all" | "tasks" | "judges" | "artifacts" | "similarity";
  let playerFilter = $state<string | null>(null);
  let typeFilter = $state<TypeFilter>("all");

  const TYPE_LABELS: { id: TypeFilter; label: string }[] = [
    { id: "all", label: "All" },
    { id: "tasks", label: "Tasks" },
    { id: "judges", label: "Judges" },
    { id: "artifacts", label: "Artifacts" },
    { id: "similarity", label: "Copy/paste" },
  ];

  function typeOf(ev: ActivityEvent): TypeFilter {
    if (ev.kind === "artifact_received") return "artifacts";
    if (ev.kind === "similarity") return "similarity";
    if (ev.kind === "task_scored" && ev.judge_name) return "judges";
    return "tasks";
  }

  /** Distinct players in feed order; the filter row appears past one. */
  const feedPlayers = $derived.by(() => {
    const seen = new Map<string, string>();
    for (const ev of events) {
      if (!seen.has(ev.player_id)) seen.set(ev.player_id, ev.player_display_name);
    }
    return [...seen.entries()].map(([id, name]) => ({ id, name }));
  });

  // Newest first. Drop empty (0-byte) artifact deliveries — they carry nothing
  // to show and only add noise. Then the user's filters.
  const orderedEvents = $derived(
    [...events]
      .reverse()
      .filter((ev) => ev.kind !== "artifact_received" || totalSize(ev) > 0)
      .filter((ev) => playerFilter === null || ev.player_id === playerFilter)
      .filter((ev) => typeFilter === "all" || typeOf(ev) === typeFilter),
  );

  // Every artifact image in the feed, flat, in display order — the lightbox
  // walks this list with ‹ › so adjacent deliveries preview without leaving
  // fullscreen. Videos keep their inline players.
  type FeedShot = { key: string; src: string; label: string };
  const feedShots = $derived.by(() => {
    const out: FeedShot[] = [];
    if (!sessionId) return out;
    for (const ev of orderedEvents) {
      if (ev.kind !== "artifact_received") continue;
      const ct = ev.detail?.content_type ?? "";
      if (!ct.startsWith("image/")) continue;
      artifactFiles(ev).forEach((f, i) => {
        const src = artifactUrl(ev, i);
        if (src) out.push({ key: `${ev.detail?.probe_id}:${i}`, src, label: f.path });
      });
    }
    return out;
  });
  let lightboxIndex = $state<number | null>(null);
  const lightboxShot = $derived(lightboxIndex !== null ? (feedShots[lightboxIndex] ?? null) : null);

  function stepLightbox(delta: number) {
    if (lightboxIndex === null || feedShots.length === 0) return;
    const n = feedShots.length;
    lightboxIndex = (lightboxIndex + delta + n) % n;
  }

  function openLightbox(key: string) {
    const index = feedShots.findIndex((s) => s.key === key);
    if (index >= 0) lightboxIndex = index;
  }

  // Absolute wall-clock time of the event, in the viewer's locale. The
  // elapsed offset says how the game unfolded; this says when it happened.
  function clockLabel(ev: ActivityEvent): string {
    const d = new Date(ev.timestamp);
    if (Number.isNaN(d.getTime())) return "";
    return d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  }

  function fullTimestamp(ev: ActivityEvent): string {
    const d = new Date(ev.timestamp);
    if (Number.isNaN(d.getTime())) return "";
    return d.toLocaleString();
  }

  function elapsedLabel(ev: ActivityEvent): string {
    if (!startedAt) return "";
    const evMs = new Date(ev.timestamp).getTime();
    const startMs = startedAt.getTime();
    if (Number.isNaN(evMs) || Number.isNaN(startMs)) return "";
    let secs = Math.max(0, Math.round((evMs - startMs) / 1000));
    const h = Math.floor(secs / 3600);
    secs -= h * 3600;
    const m = Math.floor(secs / 60);
    const s = secs - m * 60;
    if (h > 0) {
      return `${h}h ${String(m).padStart(2, "0")}m ${String(s).padStart(2, "0")}s`;
    }
    if (m > 0) {
      return `${m}m ${String(s).padStart(2, "0")}s`;
    }
    return `${s}s`;
  }

</script>

<div class="rounded-[8px] bg-white px-[24px] py-[20px]">
  <h2 class="mb-[16px] font-heading text-[16px] font-bold" style="color: #363636;">Activity</h2>

  {#if events.length > 0}
    <!-- Filters: type always; player only when the session has several. -->
    <div class="mb-[14px] flex flex-wrap items-center gap-x-4 gap-y-2">
      <div class="flex flex-wrap gap-1.5" role="group" aria-label="Filter by record type">
        {#each TYPE_LABELS as t (t.id)}
          <button
            type="button"
            onclick={() => (typeFilter = t.id)}
            data-testid="activity-filter-type-{t.id}"
            class="rounded-full px-3 py-1 text-[12px] font-semibold transition-colors
                   {typeFilter === t.id
                     ? 'bg-brand-blue text-white'
                     : 'bg-brand-light-blue text-[#5b6b86] hover:text-brand-text'}"
          >
            {t.label}
          </button>
        {/each}
      </div>
      {#if feedPlayers.length > 1}
        <div class="flex flex-wrap gap-1.5" role="group" aria-label="Filter by player">
          <button
            type="button"
            onclick={() => (playerFilter = null)}
            class="rounded-full px-3 py-1 text-[12px] font-semibold transition-colors
                   {playerFilter === null
                     ? 'bg-brand-blue text-white'
                     : 'bg-brand-light-blue text-[#5b6b86] hover:text-brand-text'}"
          >
            Everyone
          </button>
          {#each feedPlayers as p (p.id)}
            <button
              type="button"
              onclick={() => (playerFilter = playerFilter === p.id ? null : p.id)}
              data-testid="activity-filter-player-{p.name}"
              class="rounded-full px-3 py-1 text-[12px] font-semibold transition-colors
                     {playerFilter === p.id
                       ? 'bg-brand-blue text-white'
                       : 'bg-brand-light-blue text-[#5b6b86] hover:text-brand-text'}"
            >
              {p.name}
            </button>
          {/each}
        </div>
      {/if}
    </div>
  {/if}

  {#if orderedEvents.length === 0}
    <p class="text-[14px]" style="color: #5b6b86;">
      {playerFilter === null && typeFilter === 'all'
        ? 'No activity yet.'
        : 'Nothing matches the current filters.'}
    </p>
  {:else}
    <div class="flex flex-col">
      {#each orderedEvents as ev, i (ev.timestamp + ev.player_id + ev.task_id + ev.kind)}
        {@const elapsed = elapsedLabel(ev)}
        {@const avatarUrl = avatarByPlayerId.get(ev.player_id) ?? null}
        <div
          class="flex items-start gap-[12px] rounded-[6px] px-[10px] py-[14px] transition-colors hover:bg-[#f4f8fe]"
          class:border-t={i > 0}
          style="border-color: #eef3fb;"
        >
          <!-- Avatar -->
          {#if avatarUrl}
            <img
              src={ikAvatar(avatarUrl, 32)}
              alt="{ev.player_display_name} avatar"
              class="mt-[2px] h-[32px] w-[32px] shrink-0 rounded-full object-cover"
            />
          {:else}
            <span
              class="mt-[2px] inline-flex h-[32px] w-[32px] shrink-0 items-center justify-center rounded-full text-[14px] font-semibold text-white"
              style="background: #8fb4ec;"
            >
              {ev.player_display_name.charAt(0).toUpperCase()}
            </span>
          {/if}

          <!-- Name + action -->
          <div class="min-w-0 flex-grow">
            <div class="flex items-baseline gap-[6px]">
              <span class="shrink-0 text-[14px] font-semibold" style="color: #363636;">
                {ev.player_display_name}
              </span>
              {#if ev.kind === "task_started"}
                <span class="text-[14px]" style="color: #6b7280;">
                  started working on
                </span>
                <span class="text-[14px] font-medium" style="color: #363636;">
                  Task {ev.task_ordinal}
                </span>
              {:else if ev.kind === "task_scored" && ev.judge_name}
                <span class="text-[14px]" style="color: #6b7280;">
                  evaluated by
                </span>
                <span class="text-[14px] font-medium" style="color: #363636;">
                  {ev.judge_name}
                </span>
                <!-- Name the task: a judge runs once per task, so without this
                     several per-task verdicts read as identical duplicate rows. -->
                {#if ev.task_ordinal != null}
                  <span class="text-[14px]" style="color: #6b7280;">on</span>
                  <span class="text-[14px] font-medium" style="color: #363636;">
                    Task {ev.task_ordinal}
                  </span>
                {/if}
                {#if ev.point_delta != null}
                  <span
                    class="text-[14px] font-semibold tabular-nums"
                    style="color: {ev.point_delta >= 0 ? '#6be597' : '#ef4444'};"
                  >
                    {ev.point_delta >= 0 ? "+" : ""}{ev.point_delta} points
                  </span>
                {/if}
              {:else if ev.kind === "task_scored"}
                <span class="text-[14px]" style="color: #6b7280;">
                  implemented
                </span>
                <span class="text-[14px] font-medium" style="color: #363636;">
                  Task {ev.task_ordinal}
                </span>
              {:else if ev.kind === "similarity"}
                {@const pct = ev.detail?.duplicated_pct ?? 0}
                {@const src = (ev.detail?.sources ?? [])[0]}
                <span class="text-[14px]" style="color: #6b7280;">
                  copy/paste check
                </span>
                {#if ev.point_delta != null && ev.point_delta < 0}
                  <span class="text-[14px] font-medium" style="color: #363636;">
                    {pct.toFixed(0)}%{#if src}&nbsp;matches {src.player} @ <a class="underline" href="/s/{src.join_code}">{src.join_code}</a>{/if}
                  </span>
                  <span class="text-[14px] font-semibold tabular-nums" style="color: #ef4444;">
                    {ev.point_delta} points
                  </span>
                {:else}
                  <span class="text-[14px] font-medium" style="color: #6be597;">
                    clean{pct > 0 ? ` (${pct.toFixed(0)}%)` : ''}
                  </span>
                {/if}
              {:else if ev.kind === "artifact_received"}
                {@const count = artifactFiles(ev).length}
                <span class="text-[14px]" style="color: #6b7280;">
                  delivered
                </span>
                <span class="truncate text-[14px] font-medium" style="color: #363636;" title={ev.detail?.path}>
                  {count > 1 ? `${count} files` : artifactFileName(ev)}
                </span>
                <span class="shrink-0 text-[13px] tabular-nums" style="color: #5b6b86;">
                  {artifactSizeLabel(ev)}
                </span>
              {/if}
            </div>
            {#if ev.task_title && ev.kind !== "task_scored" && ev.kind !== "similarity"}
              <p class="mt-[2px] truncate text-[13px]" style="color: #5b6b86;" title={ev.task_title}>
                {ev.task_title}
              </p>
            {/if}
            {#if ev.kind === "task_scored" && !ev.judge_name && ev.point_delta && ev.point_delta > 0}
              <p class="mt-[2px] text-[13px] font-semibold" style="color: #6be597;">
                +{ev.point_delta} points
              </p>
            {/if}
            {#if ev.kind === "artifact_received"}
              {@const ct = ev.detail?.content_type ?? ""}
              {@const files = artifactFiles(ev)}
              {#if sessionId && ct.startsWith("image/")}
                <!-- Screenshots: a row of thumbnails right in the feed;
                     click opens the fullscreen preview with ‹ › navigation. -->
                <div class="mt-[6px] flex flex-wrap gap-[8px]" data-testid="artifact-image">
                  {#each files as f, i (f.path)}
                    <button
                      type="button"
                      title="{f.path} — click to enlarge"
                      onclick={() => openLightbox(`${ev.detail?.probe_id}:${i}`)}
                    >
                      <img
                        src={artifactUrl(ev, i)}
                        alt="delivered artifact {fileName(f.path)}"
                        loading="lazy"
                        class="max-h-[180px] w-auto max-w-full cursor-zoom-in rounded-[6px] border"
                        style="border-color: #e3ecf9;"
                      />
                    </button>
                  {/each}
                </div>
              {:else if sessionId && ct.startsWith("video/")}
                <!-- Screencasts: playable inline. -->
                <div class="mt-[6px] flex flex-wrap gap-[8px]" data-testid="artifact-video">
                  {#each files as f, i (f.path)}
                    <!-- svelte-ignore a11y_media_has_caption -->
                    <video
                      src={artifactUrl(ev, i)}
                      controls
                      preload="metadata"
                      title={f.path}
                      class="max-h-[220px] w-auto max-w-full rounded-[6px] border"
                      style="border-color: #e3ecf9;"
                    ></video>
                  {/each}
                </div>
              {:else}
                <!-- Anything else: say what arrived; link when fetchable. -->
                <p class="mt-[2px] text-[13px]" style="color: #5b6b86;" data-testid="artifact-info">
                  {ct || "artifact"}
                  {#if ev.detail?.within_cap === false}
                    — exceeds the requested size cap
                  {/if}
                  {#each files as f, i (f.path)}
                    · {fileName(f.path)}{#if sessionId}&nbsp;<a
                        href={artifactUrl(ev, i)}
                        target="_blank"
                        rel="noopener"
                        class="underline">download</a
                      >{/if}
                  {/each}
                </p>
              {/if}
            {/if}
            {#if ev.kind === "task_scored" && ev.detail?.criteria?.length}
              <!-- Open-ended verdict: the judge's per-criterion sheet. The
                   explanation lives in a hovercard on the chip — hover or
                   focus a parameter to read why it scored what it scored.
                   A single-criterion judge's overall feedback IS that
                   criterion's story, so it fills in when no rationale came. -->
              {@const anyRationale = ev.detail.criteria.some((x) => x.rationale)}
              <div class="mt-[4px] flex flex-wrap gap-[6px]" data-testid="verdict-criteria">
                {#each ev.detail.criteria as c (c.key)}
                  {@const note = c.rationale || (!anyRationale ? ev.detail.feedback : null)}
                  <span class="group relative inline-flex">
                    <span
                      class="inline-flex items-center gap-[4px] rounded-full px-[10px] py-[3px] text-[12px] font-medium {note ? 'cursor-help' : ''}"
                      style="background: #f0f5fd; color: #4b5f7d;"
                    >
                      {c.key}
                      {#if c.score != null}
                        <span
                          class="font-semibold tabular-nums"
                          style="color: {c.score >= 7 ? '#16a34a' : c.score >= 4 ? '#d97706' : '#dc2626'};"
                        >
                          {c.score.toFixed(1)}
                        </span>
                      {:else}
                        <span style="color: #7b8aa3;">n/a</span>
                      {/if}
                    </span>
                    {#if note}
                      <!-- Screen readers get the text inline; sighted users
                           hover the chip. -->
                      <span class="sr-only">{note}</span>
                      <span
                        aria-hidden="true"
                        class="pointer-events-none invisible absolute bottom-full left-0 z-20 mb-[6px] w-max max-w-[360px] rounded-lg bg-white p-[12px] opacity-0 shadow-[0_6px_24px_0_rgba(19,101,218,0.18)] transition-opacity duration-100 group-hover:visible group-hover:opacity-100"
                        data-testid="criterion-hovercard"
                      >
                        <!-- Rationales quote files and symbols in backticks
                             just as the long-form verdicts do. -->
                        <MarkdownContent
                          value={note}
                          class="verdict-md !text-[13px] !leading-[1.55]"
                        />
                      </span>
                    {/if}
                  </span>
                {/each}
              </div>
            {/if}
            {#if ev.kind === "task_scored" && ev.detail?.feedback && !ev.detail?.criteria?.length}
              <!-- A verdict without a criteria sheet (anti-cheat, execution
                   judges): the written comment has no chip to live on, so it
                   stays inline. -->
              <div
                class="mt-[6px] border-l-2 pl-[10px]"
                style="border-color: #e3ecf9;"
                data-testid="verdict-feedback"
              >
                <ClampedText text={ev.detail.feedback} />
              </div>
            {/if}
          </div>

          <!-- When it happened (wall clock) + how far into the session -->
          <span
            class="mt-[4px] flex shrink-0 flex-col items-end font-mono text-[12px] tabular-nums"
            style="color: #7b8aa3;"
            title={fullTimestamp(ev)}
          >
            <span style="color: #5b6b86;">{clockLabel(ev) || "—"}</span>
            {#if elapsed}
              <span>+{elapsed}</span>
            {/if}
          </span>
        </div>
      {/each}
    </div>
  {/if}

  {#if lightboxShot}
    <ImageLightbox
      src={lightboxShot.src}
      alt={lightboxShot.label}
      counter={feedShots.length > 1 ? `${(lightboxIndex ?? 0) + 1} / ${feedShots.length}` : null}
      onprev={feedShots.length > 1 ? () => stepLightbox(-1) : null}
      onnext={feedShots.length > 1 ? () => stepLightbox(1) : null}
      onclose={() => (lightboxIndex = null)}
    />
  {/if}
</div>