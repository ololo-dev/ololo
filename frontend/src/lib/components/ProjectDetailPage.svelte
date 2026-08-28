<script lang="ts">
  import { untrack } from "svelte";
  import type {
    Project,
    ProjectJudge,
    ProjectPart,
    Session,
    TaskPreviewItem,
    TopPlayersBoards,
  } from "$lib/api";
  import { WsProjectClient } from "$lib/ws-project.svelte";
  import ProjectHeroCard from "$lib/components/projects/ProjectHeroCard.svelte";
  import SessionList from "$lib/components/projects/SessionList.svelte";
  import ProjectTopPlayers from "$lib/components/projects/ProjectTopPlayers.svelte";
  import StartSessionPopup from "$lib/components/projects/StartSessionPopup.svelte";
  import LiveSessions from "$lib/components/projects/LiveSessions.svelte";
  import ProjectTaskList from "$lib/components/projects/ProjectTaskList.svelte";
  import CampaignPartsList from "$lib/components/projects/CampaignPartsList.svelte";
  import CampaignPartNav from "$lib/components/projects/CampaignPartNav.svelte";

  interface Props {
    project: Project;
    /** SSR-seeded sessions; kept live via WebSocket. */
    ssrSessions: Session[];
    judges: ProjectJudge[];
    topPlayers: TopPlayersBoards;
    /** The task arc; empty when the project hides it (show_tasks: false). */
    taskPreview?: TaskPreviewItem[];
    /** Campaign parts: this project's own when it is a campaign parent, its
     *  siblings' when it is a part (that list carries this part's lock
     *  state). Empty for standalone projects. */
    parts?: ProjectPart[];
    message: string | null;
    currentUserId: string | null | undefined;
    isAdmin: boolean;
  }

  let {
    project,
    ssrSessions,
    judges,
    topPlayers,
    taskPreview = [],
    parts = [],
    message,
    currentUserId,
    isAdmin,
  }: Props = $props();

  const isCampaign = $derived((project.part_count ?? 0) > 0);
  // Where a campaign wants the viewer to go next: the part they are in the
  // middle of, else the first one open to them. Null when the ladder is
  // finished — or when this is not a campaign at all.
  const nextPart = $derived.by(() => {
    if (!isCampaign) return null;
    return (
      parts.find((p) => p.state === "in_progress") ?? parts.find((p) => p.state === "available") ?? null
    );
  });
  // This project's own entry in its campaign — the source of its lock state
  // and the "Part N of …" chip.
  const ownPart = $derived(
    project.parent_project_id ? parts.find((p) => p.id === project.id) : undefined,
  );
  // Only a signed-in viewer gets a real answer: for anonymous visitors the
  // server reports every part past the first as locked, and telling a browser
  // "locked" when the truth is "we don't know you yet" would be a lie.
  const lockedReason = $derived.by(() => {
    if (!currentUserId || ownPart?.state !== "locked") return null;
    const index = parts.findIndex((p) => p.id === project.id);
    const previous = index > 0 ? parts[index - 1] : undefined;
    return previous
      ? `Finish “${previous.name}” to unlock this part.`
      : "This part is not open yet.";
  });

  type Tab = "parts" | "tasks" | "sessions" | "players";
  // A campaign opens on its parts — that ladder is the whole page. Otherwise
  // tasks lead when the project shows its arc; projects that hide the ladder
  // open on Top Players.
  function defaultTab(): Tab {
    if ((project.part_count ?? 0) > 0) return "parts";
    return taskPreview.length > 0 ? "tasks" : "players";
  }

  // SvelteKit reuses this component across /projects/[slug] navigations, so
  // anything seeded once at mount leaks into the next project. That is how
  // clicking a campaign's part left the campaign's card grid rendered over
  // it — the tab was still "parts" — and how the session list kept showing
  // the project you came from. Everything per-project is keyed on the id and
  // re-seeded when it changes.
  let shownProjectId = $state(untrack(() => project.id));
  /** The tab the viewer picked here; null means "whatever this project opens on". */
  let chosenTab = $state<Tab | null>(null);
  // Mutable reactive copy seeded from SSR; WS updates drive subsequent mutations.
  let sessions = $state(untrack(() => [...ssrSessions]));
  let showStartPopup = $state(false);
  // Which project the popup starts. A campaign page starts its *parts*, so the
  // popup cannot assume the project the page is about.
  let startSlug = $state<string | null>(null);

  function openStart(slug: string): void {
    startSlug = slug;
    showStartPopup = true;
  }
  let showCancelled = $state(false);

  const activeTab = $derived(chosenTab ?? defaultTab());

  $effect(() => {
    if (shownProjectId === project.id) return;
    shownProjectId = project.id;
    chosenTab = null;
    sessions = [...ssrSessions];
    showStartPopup = false;
    startSlug = null;
    showCancelled = false;
  });

  // WS client stored so the template can read reactive client state
  // (e.g. sessionCountdowns) without mirroring it.
  let wsClient = $state<WsProjectClient | null>(null);

  // Wire up project-level WS observer. $effect runs only in the browser.
  $effect(() => {
    const client = new WsProjectClient(project.id, (frame) => {
      const idx = sessions.findIndex((s) => s.id === frame.session_id);
      if (idx >= 0) {
        sessions[idx] = {
          ...sessions[idx],
          name: frame.name,
          status: frame.status,
          // Older servers omit the count; keep the last known value rather
          // than flashing the card back to zero.
          player_count: frame.player_count ?? sessions[idx].player_count,
        };
      } else {
        sessions = [
          ...sessions,
          {
            id: frame.session_id,
            name: frame.name,
            status: frame.status,
            owner_id: null,
            project_id: frame.project_id,
            created_at: frame.created_at,
            join_code: frame.join_code,
            player_count: frame.player_count,
          },
        ];
      }
    });
    wsClient = client;
    client.connect();
    return () => {
      client.disconnect();
      wsClient = null;
    };
  });

  // Status ordering: lobby=0, running=1, finished=2, cancelled=3 (hidden by default)
  const STATUS_ORDER: Record<string, number> = {
    lobby: 0,
    running: 1,
    finished: 2,
    cancelled: 3,
  };

  const STATUS_LABEL: Record<string, string> = {
    lobby: "Lobby",
    running: "Ongoing",
    finished: "Completed",
    cancelled: "Cancelled / Failed",
  };

  type SessionGroup = { label: string; status: string; sessions: Session[] };

  const sessionGroups = $derived.by<SessionGroup[]>(() => {
    const visible = ["lobby", "running", "finished"];
    if (showCancelled) visible.push("cancelled");

    const buckets = new Map<string, Session[]>();
    for (const s of sessions) {
      const key = STATUS_ORDER[s.status] !== undefined ? s.status : "cancelled";
      if (!visible.includes(key)) continue;
      if (!buckets.has(key)) buckets.set(key, []);
      buckets.get(key)!.push(s);
    }

    return visible
      .filter((k) => buckets.has(k))
      .map((k) => ({
        label: STATUS_LABEL[k] ?? k,
        status: k,
        sessions: buckets
          .get(k)!
          .sort(
            (a, b) =>
              new Date(b.created_at).getTime() -
              new Date(a.created_at).getTime(),
          ),
      }));
  });

  const totalVisible = $derived(
    sessionGroups.reduce((n, g) => n + g.sessions.length, 0),
  );

  /** Derived from live session list — updates as WS frames arrive. */
  const hasActiveSessions = $derived(
    sessions.some((s) => s.status === "lobby" || s.status === "running"),
  );

  /**
   * Sessions you can still act on — running (watch) and lobby (join before it
   * starts) — spotlighted above the tabs. Running first, then newest, so the
   * card you can watch right now leads.
   */
  const liveSessions = $derived(
    sessions
      .filter((s) => s.status === "running" || s.status === "lobby")
      .sort((a, b) => {
        if (a.status !== b.status) return a.status === "running" ? -1 : 1;
        return (
          new Date(b.created_at).getTime() - new Date(a.created_at).getTime()
        );
      }),
  );
</script>

<!-- Full-bleed wrapper -->
<div class="-mx-6 -mt-8 min-h-screen bg-brand-light-blue">
  <div class="mx-auto w-full max-w-[1206px] px-[18px] py-[88px]">
    <ProjectHeroCard
      {project}
      {judges}
      {currentUserId}
      {isAdmin}
      sessionCount={sessions.length}
      {hasActiveSessions}
      {lockedReason}
      {nextPart}
      onStart={() => openStart(project.slug ?? "")}
      onStartPart={openStart}
    />

    <!-- A part page is an ordinary project page; this strip is the only
         campaign furniture on it — where the part sits, and the way back and
         forward without a detour through the campaign. -->
    {#if project.parent_project_id && parts.length > 0}
      <CampaignPartNav
        {parts}
        currentId={project.id}
        campaignSlug={project.parent_project_slug ?? null}
        campaignName={project.parent_project_name ?? "the campaign"}
      />
    {/if}

    <!-- Archived message banner -->
    {#if message === "archived"}
      <div
        class="mt-[24px] rounded-[8px] border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-800"
      >
        This project is archived. Editing is unavailable until it is unarchived.
      </div>
    {/if}

    <!-- Live sessions spotlight, above the tabs. A campaign hosts none of
         its own — they belong to its parts — so the strip stays off. -->
    {#if !isCampaign}
      <LiveSessions sessions={liveSessions} {wsClient} />
    {/if}

    <!-- Tabs: Tasks / Top Players / Sessions -->
    <div class="mt-[48px] border-b border-[#dce9fc]">
      <div class="flex gap-[8px]" role="tablist" aria-label="Project details">
        {#if isCampaign}
          <button
            type="button"
            role="tab"
            id="tab-parts"
            aria-controls="panel-parts"
            aria-selected={activeTab === "parts"}
            onclick={() => (chosenTab = "parts")}
            class="relative -mb-px px-[4px] pb-[12px] font-heading text-[20px] font-bold transition-colors
              {activeTab === 'parts' ? 'text-brand-text' : 'text-brand-muted hover:text-brand-text'}"
          >
            Parts
            <span class="text-brand-muted">({parts.length})</span>
            {#if activeTab === "parts"}
              <span class="absolute inset-x-0 bottom-0 h-[3px] rounded-t bg-brand-blue"></span>
            {/if}
          </button>
        {/if}
        {#if taskPreview.length > 0}
          <button
            type="button"
            role="tab"
            id="tab-tasks"
            aria-controls="panel-tasks"
            aria-selected={activeTab === "tasks"}
            onclick={() => (chosenTab = "tasks")}
            class="relative -mb-px px-[4px] pb-[12px] font-heading text-[20px] font-bold transition-colors
              {activeTab === 'tasks' ? 'text-brand-text' : 'text-brand-muted hover:text-brand-text'}"
          >
            Tasks
            <span class="text-brand-muted">({taskPreview.length})</span>
            {#if activeTab === "tasks"}
              <span class="absolute inset-x-0 bottom-0 h-[3px] rounded-t bg-brand-blue"></span>
            {/if}
          </button>
        {/if}
        <button
          type="button"
          role="tab"
          id="tab-players"
          aria-controls="panel-players"
          aria-selected={activeTab === "players"}
          onclick={() => (chosenTab = "players")}
          class="relative -mb-px px-[4px] pb-[12px] font-heading text-[20px] font-bold transition-colors
            {activeTab === 'players' ? 'text-brand-text' : 'text-brand-muted hover:text-brand-text'}"
        >
          Top Players
          <span class="text-brand-muted">({topPlayers.players.length})</span>
          {#if activeTab === "players"}
            <span class="absolute inset-x-0 bottom-0 h-[3px] rounded-t bg-brand-blue"></span>
          {/if}
        </button>
        <!-- No Sessions tab on a campaign: `ololo start <campaign>` is
             refused, so the list could only ever be an empty one. -->
        {#if !isCampaign}
          <button
            type="button"
            role="tab"
            id="tab-sessions"
            aria-controls="panel-sessions"
            aria-selected={activeTab === "sessions"}
            onclick={() => (chosenTab = "sessions")}
            class="relative -mb-px px-[4px] pb-[12px] font-heading text-[20px] font-bold transition-colors
              {activeTab === 'sessions' ? 'text-brand-text' : 'text-brand-muted hover:text-brand-text'}"
          >
            Sessions
            <span class="text-brand-muted">({totalVisible})</span>
            {#if activeTab === "sessions"}
              <span class="absolute inset-x-0 bottom-0 h-[3px] rounded-t bg-brand-blue"></span>
            {/if}
          </button>
        {/if}
      </div>
    </div>

    {#if activeTab === "parts" && isCampaign}
      <!-- The campaign ladder: the parts, in order, with the viewer's own
           progression on each. -->
      <div id="panel-parts" role="tabpanel" aria-labelledby="tab-parts">
        <CampaignPartsList {parts} signedIn={!!currentUserId} onStart={openStart} />
      </div>
    {:else if activeTab === "tasks"}
      <!-- The task arc a player signs up for (audit UI-H2) -->
      <div id="panel-tasks" role="tabpanel" aria-labelledby="tab-tasks">
        <ProjectTaskList tasks={taskPreview} />
      </div>
    {:else if activeTab === "sessions" && !isCampaign}
      <div id="panel-sessions" role="tabpanel" aria-labelledby="tab-sessions">
        <SessionList
          {sessions}
          {sessionGroups}
          {isAdmin}
          bind:showCancelled
          {wsClient}
        />
      </div>
    {:else}
      <div id="panel-players" role="tabpanel" aria-labelledby="tab-players">
        <ProjectTopPlayers
          players={topPlayers.players}
          seasonPlayers={topPlayers.season_players}
          seasonStart={topPlayers.season_start}
          partsTotal={topPlayers.parts_total ?? null}
        />
      </div>
    {/if}
  </div>
</div>

<!-- Start session popup -->
<StartSessionPopup slug={startSlug ?? project.slug ?? ""} bind:open={showStartPopup} />

