<script lang="ts">
  import { browser } from "$app/environment";
  import { page } from "$app/state";
  import { untrack } from "svelte";
  import CodeBlock from "$lib/components/CodeBlock.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import {
    ApiError,
    createSession,
    getProjectBySlug,
    listProjectSessions,
    patchSession,
    type Project,
    type Session,
  } from "$lib/api";

  type Mode = "console" | "browser";
  let {
    slug,
    open = $bindable(),
    startOn = "console",
    project: given = null,
  }: {
    slug: string;
    open: boolean;
    /** The tab an opening lands on. A personal project's page shows the
     *  console line itself, so its popup opens on hosting. */
    startOn?: Mode;
    /** The project behind `slug`, when the caller already holds it: whether
     *  hosting is offered depends on what it is and whose it is, and the
     *  first paint should know. Without it an admin's popup reads it by slug. */
    project?: Project | null;
  } = $props();

  // An admin may host several sessions at once straight from here — the
  // server lets them, and hosting holds no player row. So may the owner of
  // a personal project: a free-form project is what a workshop runs on,
  // one session per group. Everybody else starts (and plays) one session
  // from the console.
  const isAdmin = $derived(page.data?.isAdmin === true);
  const myId = $derived((page.data?.user as { id?: string } | null | undefined)?.id ?? null);
  let loadedProject = $state<Project | null>(null);
  const project = $derived(given ?? loadedProject);
  const ownsPersonal = $derived(
    project?.kind === "personal" && myId !== null && project.owner_user_id === myId,
  );
  const canHost = $derived(isAdmin || ownsPersonal);

  // Playing from the console is how a session starts, for admins too; the
  // browser tab is the extra for running several at once. Every opening
  // lands on `startOn`.
  let mode = $state<Mode>("console");
  $effect(() => {
    if (!open) return;
    untrack(() => {
      mode = startOn;
      // What the last opening left behind is not this one's.
      error = null;
      fresh = null;
      confirming = null;
    });
  });
  const hosting = $derived(canHost && mode === "browser");

  const LIVE = new Set(["lobby", "running", "paused"]);
  const projectId = $derived(project?.id ?? null);
  const projectName = $derived(project?.name ?? "");
  // The admin's own live sessions of this project, oldest first — read
  // from the server, so the list survives closing the popup or a reload.
  let live = $state<Session[]>([]);
  let loaded = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let fresh = $state<string | null>(null);
  let confirming = $state<string | null>(null);
  let copied = $state<string | null>(null);

  const ERRORS: Record<string, string> = {
    campaign_project: "A campaign hosts no sessions: start one of its parts.",
    part_locked: "This part opens once the part before it is completed.",
    project_archived: "The project is archived.",
    personal_project: "Only the owner of a personal project starts its sessions.",
    project_forbidden: "This project is private.",
    too_many_hosted_sessions: "You already host as many live sessions as one account can. End one to create another.",
  };
  function describe(err: unknown, what: string): string {
    if (err instanceof ApiError) {
      return ERRORS[err.code ?? ""] ?? `Could not ${what} (${err.code ?? err.status}).`;
    }
    return `Could not ${what}: ${err instanceof Error ? err.message : String(err)}`;
  }

  async function refresh(): Promise<void> {
    if (!projectId) return;
    const all = await listProjectSessions(projectId);
    live = all
      .filter((s) => s.owner_id === myId && LIVE.has(s.status) && s.join_code)
      .sort((a, b) => a.created_at.localeCompare(b.created_at));
    loaded = true;
  }

  // Without the project in hand, an admin's popup reads it on every opening
  // of a new slug — the hosting list needs its id and name. Nobody else
  // can host a project they were not handed, so nobody else pays the
  // round-trip.
  let lastSlug: string | null = null;
  $effect(() => {
    if (!open || !slug || given || !isAdmin || slug === lastSlug) return;
    let stopped = false;
    const current = slug;
    // Only open/slug drive this; the state the loader writes must not.
    untrack(() => {
      loadedProject = null;
      loaded = false;
      live = [];
      error = null;
      void (async () => {
        try {
          const fetched = await getProjectBySlug(current);
          if (stopped) return;
          loadedProject = fetched;
          lastSlug = current;
        } catch (err) {
          if (!stopped) error = describe(err, "load the project");
        }
      })();
    });
    return () => {
      stopped = true;
    };
  });

  // While hosting, keep statuses and player counts current: a lobby turns
  // into a running session within a minute.
  $effect(() => {
    if (!open || !hosting || !projectId) return;
    let stopped = false;
    untrack(() => {
      refresh().catch((err) => {
        if (!stopped) error = describe(err, "load your sessions");
      });
    });
    const timer = setInterval(() => {
      if (!stopped) refresh().catch(() => {});
    }, 5000);
    return () => {
      stopped = true;
      clearInterval(timer);
    };
  });

  async function host(): Promise<void> {
    if (busy || !projectId) return;
    busy = true;
    error = null;
    try {
      // Named after the project, like `ololo start` names it — but hosted,
      // which `ololo start` never is: the server lets a host have several.
      const session = await createSession(projectName, projectId, { host: true });
      fresh = session.join_code ?? null;
      await refresh();
    } catch (err) {
      error = describe(err, "create the session");
    } finally {
      busy = false;
    }
  }

  async function cancel(s: Session): Promise<void> {
    if (confirming !== s.id) {
      confirming = s.id;
      return;
    }
    confirming = null;
    error = null;
    try {
      await patchSession(s.id, { status: "cancelled" });
      await refresh();
    } catch (err) {
      error = describe(err, "cancel the session");
    }
  }

  async function copy(key: string, text: string): Promise<void> {
    if (!browser) return;
    try {
      await navigator.clipboard.writeText(text);
      copied = key;
      setTimeout(() => {
        if (copied === key) copied = null;
      }, 1500);
    } catch {
      // clipboard unavailable
    }
  }

  const allCommands = $derived(live.map((s) => `ololo join ${s.join_code}`).join("\n"));

  const STATUS: Record<string, { label: string; cls: string }> = {
    lobby: { label: "Lobby", cls: "bg-amber-100 text-amber-700" },
    running: { label: "Running", cls: "bg-green-100 text-green-700" },
    paused: { label: "Paused", cls: "bg-brand-light text-brand-muted" },
  };
  function players(n: number | undefined): string {
    const c = n ?? 0;
    return c === 1 ? "1 player" : `${c} players`;
  }
</script>

{#if open && slug}
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
    role="dialog"
    aria-modal="true"
    aria-label="Start session"
  >
    <div
      class="popup-card relative max-h-[calc(100vh-32px)] w-full max-w-[560px] overflow-y-auto rounded-[8px] bg-white px-6 py-8 sm:px-[48px] sm:py-[40px]"
    >
      <!-- Close button -->
      <button
        type="button"
        onclick={() => (open = false)}
        aria-label="Close"
        class="absolute right-[20px] top-[20px] flex h-[32px] w-[32px] items-center justify-center rounded-full text-brand-muted transition-opacity hover:opacity-60"
      >
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="20"
          height="20"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2.5"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <line x1="18" y1="6" x2="6" y2="18" /><line
            x1="6"
            y1="6"
            x2="18"
            y2="18"
          />
        </svg>
      </button>

      <h3
        class="mb-[8px] font-heading text-[26px] font-bold leading-[1.23] text-brand-text"
      >
        Start session
      </h3>

      {#if canHost}
        <div
          class="mb-[20px] mt-[14px] grid grid-cols-2 rounded-[10px] border border-brand-border bg-brand-blue/5 p-[4px]"
          role="tablist"
          aria-label="Where to start it"
        >
          {#each [{ id: "console", label: "Play from the console" }, { id: "browser", label: "Host several sessions" }] as t (t.id)}
            <button
              type="button"
              role="tab"
              aria-selected={mode === t.id}
              onclick={() => (mode = t.id as Mode)}
              class="rounded-[7px] px-[12px] py-[8px] text-[14px] font-bold transition-colors {mode ===
              t.id
                ? 'bg-white text-brand-blue shadow-[0_1px_3px_rgba(0,0,0,0.12)]'
                : 'text-brand-muted hover:text-brand-text'}"
            >
              {t.label}
            </button>
          {/each}
        </div>
      {/if}

      <!-- Above both views: a project that failed to load leaves an admin on
           the console view, and that is where they must learn why. -->
      {#if error}
        <p class="mb-[14px] text-[14px] text-brand-red" role="alert">{error}</p>
      {/if}

      {#if hosting}
        <section data-testid="host-from-web">
          <p class="mb-[20px] text-[15px] text-brand-muted">
            One session per group: you host, they play. A lobby counts down from the moment it is
            created, and latecomers can still join once it runs.
          </p>

          <Button
            type="button"
            variant="primary"
            size="full"
            disabled={busy || !projectId}
            onclick={() => void host()}
          >
            {busy ? "Creating…" : live.length > 0 ? "+ Create another session" : "Create session"}
          </Button>

          {#if live.length > 0}
            <div class="mb-[10px] mt-[28px] flex items-baseline justify-between gap-[12px]">
              <h4 class="font-heading text-[16px] font-bold text-brand-text">
                Your live sessions <span class="text-brand-muted">· {live.length}</span>
              </h4>
              {#if live.length > 1}
                <button
                  type="button"
                  onclick={() => void copy("all", allCommands)}
                  class="shrink-0 text-[14px] font-bold text-brand-blue hover:underline"
                >
                  {copied === "all" ? "Copied ✓" : "Copy all join commands"}
                </button>
              {/if}
            </div>
            <ul class="flex flex-col gap-[8px]" aria-label="Your live sessions">
              {#each live as s (s.id)}
                {@const st = STATUS[s.status] ?? STATUS.lobby}
                <li
                  class="flex flex-wrap items-center justify-between gap-x-[14px] gap-y-[10px] rounded-[10px] border px-[16px] py-[12px] transition-colors {fresh ===
                  s.join_code
                    ? 'border-brand-blue bg-brand-blue/5'
                    : 'border-brand-border'}"
                  data-testid="hosted-session"
                >
                  <div class="min-w-0">
                    <div
                      class="font-mono text-[18px] font-bold leading-tight tracking-[0.08em] text-brand-text"
                      aria-label="Join code {s.join_code}"
                    >
                      {s.join_code}
                    </div>
                    <div class="mt-[4px] flex items-center gap-[8px] text-[13px]">
                      <span class="rounded-full px-[8px] py-[1px] font-bold {st.cls}">{st.label}</span>
                      <span class="text-brand-muted">{players(s.player_count)}</span>
                    </div>
                  </div>
                  <div class="flex flex-col items-end gap-[6px] text-[14px] font-bold">
                    <div class="flex items-center gap-[16px]">
                      <button
                        type="button"
                        onclick={() => void copy(s.id, `ololo join ${s.join_code}`)}
                        title="Copy “ololo join {s.join_code}”"
                        class="rounded-full border border-brand-blue px-[12px] py-[3px] text-brand-blue transition-colors hover:bg-brand-blue hover:text-white"
                      >
                        {copied === s.id ? "Copied ✓" : "Copy command"}
                      </button>
                      <a
                        href={s.status === "lobby" ? `/s/${s.join_code}/lobby` : `/s/${s.join_code}`}
                        target="_blank"
                        rel="noopener"
                        class="text-brand-blue hover:underline"
                      >
                        Open ↗
                      </a>
                    </div>
                    <button
                      type="button"
                      onclick={() => void cancel(s)}
                      onblur={() => {
                        if (confirming === s.id) confirming = null;
                      }}
                      class="text-[13px] {confirming === s.id
                        ? 'text-brand-red'
                        : 'font-normal text-brand-muted hover:text-brand-red'}"
                    >
                      {confirming === s.id ? "Click again to cancel" : "Cancel session"}
                    </button>
                  </div>
                </li>
              {/each}
            </ul>
          {:else if loaded}
            <p class="mt-[16px] text-center text-[14px] text-brand-muted">
              No live sessions of this project yet.
            </p>
          {/if}
        </section>
      {:else}
        <p class="mb-[24px] text-[15px] text-brand-muted">
          Enter this command in your console to start the session.
        </p>

        <CodeBlock code="ololo start {slug}" />
      {/if}
    </div>
  </div>
{/if}
