<script lang="ts">
  // "Play your own work" on the landing: the visitor describes the work,
  // ololo drafts a project from it (a name and a navigation map, as the
  // "Suggest tasks" of /projects/new does) and a popup offers to start it
  // as is — create it and show the command that starts a session — or to
  // edit it first in the full form. A visitor who is not signed in signs up
  // first; the description waits in this browser, and the draft follows the
  // sign-in on its own.
  import { onMount, untrack } from "svelte";
  import { goto } from "$app/navigation";
  import Modal from "$lib/components/ui/Modal.svelte";
  import CodeBlock from "$lib/components/CodeBlock.svelte";
  import {
    ApiError,
    createPersonalProject,
    getPersonalProjectOptions,
    suggestPersonalProject,
  } from "$lib/api";
  import type { PersonalProjectOptions, Project } from "$lib/api";
  import { formatDuration } from "$lib/format";
  import { saveProjectDraft } from "$lib/project-draft";

  let {
    isAuthenticated,
    onSignIn,
  }: {
    isAuthenticated: boolean;
    /** Opens sign-up for a visitor who is not signed in. */
    onSignIn: () => void;
  } = $props();

  const DESCRIPTION_KEY = "ololo.ownProject.description";
  const PENDING_KEY = "ololo.ownProject.pending";
  /** A sign-in that took longer than this was not on the way to a draft. */
  const PENDING_TTL_MS = 30 * 60 * 1000;

  interface Draft {
    name: string;
    tasks: { title: string; description: string }[];
    /** Why the map is empty, when it is. */
    note: string | null;
  }

  let description = $state("");
  let analyzing = $state(false);
  let error = $state<string | null>(null);
  let options = $state<PersonalProjectOptions | null>(null);
  let draft = $state<Draft | null>(null);
  let creating = $state(false);
  let createError = $state<string | null>(null);
  let created = $state<Project | null>(null);

  function stored(key: string): string | null {
    try {
      return localStorage.getItem(key);
    } catch {
      return null;
    }
  }
  function store(key: string, value: string | null) {
    try {
      if (value === null) localStorage.removeItem(key);
      else localStorage.setItem(key, value);
    } catch {
      // Storage refused: the description simply is not kept.
    }
  }
  /** Whether a sign-in started from this block is waiting for its draft;
   *  reading it clears it. */
  function takePending(): boolean {
    const at = Number(stored(PENDING_KEY));
    store(PENDING_KEY, null);
    return Number.isFinite(at) && at > 0 && Date.now() - at < PENDING_TTL_MS;
  }

  onMount(() => {
    const kept = stored(DESCRIPTION_KEY);
    if (kept && !description) description = kept;
  });

  // Signed in with a draft waiting — in this page through the sign-in
  // popup, or back from a GitHub or Google sign-in: draft it now.
  $effect(() => {
    if (!isAuthenticated) return;
    untrack(() => {
      if (takePending()) void analyze();
    });
  });

  /** The name the server gives a project without one: its description's
   *  first line, cut at a word. */
  function nameFrom(text: string): string {
    const line =
      text
        .split("\n")
        .map((l) => l.trim().replace(/^[#\-*> ]+/, "").trim())
        .find((l) => l.length > 0) ?? "";
    if (!line) return "My project";
    if (line.length <= 60) return line;
    const cut = line.slice(0, 60);
    const space = cut.lastIndexOf(" ");
    return `${(space > 30 ? cut.slice(0, space) : cut).replace(/[,.;: ]+$/, "")}…`;
  }

  function submit(event: SubmitEvent) {
    event.preventDefault();
    error = null;
    const text = description.trim();
    if (!text) return;
    if (!isAuthenticated) {
      store(DESCRIPTION_KEY, text);
      store(PENDING_KEY, String(Date.now()));
      onSignIn();
      return;
    }
    void analyze();
  }

  async function analyze() {
    const text = description.trim();
    if (!text || analyzing) return;
    error = null;
    createError = null;
    created = null;
    analyzing = true;
    try {
      const opts = options ?? (await getPersonalProjectOptions());
      options = opts;
      if (!opts.creation_allowed) {
        error = "Project creation is not enabled for your account.";
        return;
      }
      if (text.length > opts.limits.max_description_chars) {
        error = `Keep it under ${opts.limits.max_description_chars} characters — the details can go into the tasks.`;
        return;
      }
      let name: string | null = null;
      let tasks: Draft["tasks"] = [];
      let note: string | null = null;
      if (opts.suggest_available) {
        try {
          const suggested = await suggestPersonalProject(text, undefined);
          name = suggested.name;
          tasks = suggested.tasks
            .slice(0, opts.limits.max_tasks)
            .map((t) => ({ title: t.title, description: t.description ?? "" }));
        } catch (e) {
          if (e instanceof ApiError && e.code === "rate_limited") {
            error = "Too many drafts in a row — try again in a few minutes.";
            return;
          }
          note =
            "ololo could not split it into tasks this time: it plays as one task. Edit the project to split it yourself.";
        }
      } else {
        note =
          "This server drafts no tasks: the project plays as one task. Edit it to split it into several.";
      }
      draft = { name: (name ?? nameFrom(text)).slice(0, opts.limits.max_name_chars), tasks, note };
    } catch {
      error = "Could not read your description right now. Try again in a moment.";
    } finally {
      analyzing = false;
    }
  }

  const defaultJudges = $derived(options ? options.judges.filter((j) => j.default) : []);

  function request() {
    if (!draft || !options) return null;
    return {
      name: draft.name,
      description: description.trim(),
      tasks: draft.tasks,
      judges: defaultJudges.map((j) => j.slug),
      session_duration_secs: options.session.default_secs,
      public: true,
      repo_url: "",
      repo_ref: "",
    };
  }

  function failure(e: unknown): string {
    if (!(e instanceof ApiError)) return "Could not create the project. Please try again.";
    const body = (e.body ?? {}) as { detail?: unknown };
    if (e.status === 403) return "Project creation is not enabled for your account.";
    if (e.code === "no_judges_available") return "No judges are set up on this instance yet.";
    if (e.code === "invalid_personal_project" && typeof body.detail === "string") {
      return `${body.detail} — edit the project to fix it.`;
    }
    return "Could not create the project. Please try again, or edit it first.";
  }

  async function start() {
    const req = request();
    if (!req || creating) return;
    createError = null;
    creating = true;
    try {
      created = await createPersonalProject({
        ...req,
        tasks: req.tasks.map((t) => (t.description ? t : { title: t.title })),
      });
      // The work is a project now: the block starts over.
      description = "";
      store(DESCRIPTION_KEY, null);
    } catch (e) {
      createError = failure(e);
    } finally {
      creating = false;
    }
  }

  function edit() {
    const req = request();
    if (!req) return;
    saveProjectDraft(req);
    void goto("/projects/new?draft=1");
  }

  function close() {
    draft = null;
    created = null;
    createError = null;
  }
</script>

<section
  class="mb-10 rounded-[8px] bg-white px-6 py-8 md:mb-16 md:px-10 md:py-10"
  data-testid="own-project"
>
  <h3 class="font-heading text-[22px] font-bold text-[#363636] md:text-[28px]">
    Play your own work
  </h3>
  <p class="mt-2 max-w-[760px] text-[15px] leading-relaxed text-[#6b7686]">
    Describe what you want to build or change in your project. ololo drafts a project from it —
    tasks in the order you would do them, each one reviewed by judges — and you play it as a
    session in your own repository.
  </p>
  <form class="mt-6" onsubmit={submit}>
    <label for="own-project-description" class="sr-only">What do you want to build?</label>
    <textarea
      id="own-project-description"
      rows="4"
      bind:value={description}
      placeholder="Add a CSV export to the reports page. It must honour the table's filters and include the totals row…"
      data-testid="own-project-description"
      class="w-full rounded-[8px] border-2 border-brand-border bg-white px-4 py-3 text-base text-brand-text placeholder:text-brand-muted focus:border-brand-blue focus:outline-none"
    ></textarea>
    <div class="mt-4 flex flex-col gap-3 sm:flex-row sm:items-center">
      <button
        type="submit"
        disabled={analyzing || description.trim().length === 0}
        data-testid="own-project-suggest"
        class="w-full rounded-md bg-[#0269fb] px-6 py-3 text-base font-semibold text-white transition-colors hover:bg-blue-700 disabled:cursor-not-allowed disabled:opacity-40 sm:w-auto"
      >
        {analyzing ? "Reading your description…" : "Suggest a project"}
      </button>
      {#if !isAuthenticated}
        <p class="text-sm text-[#6b7686]">
          You sign up first; your description stays here.
        </p>
      {/if}
    </div>
    {#if error}
      <p class="mt-3 text-sm text-red-500" data-testid="own-project-error">{error}</p>
    {/if}
  </form>
</section>

<Modal open={draft !== null} onClose={close} maxWidth="md">
  {#if created}
    <div data-testid="own-project-created">
      <h3 class="font-heading text-[26px] font-bold leading-[1.23] text-brand-text">
        {created.name} is ready
      </h3>
      <p class="mb-5 mt-2 text-[15px] text-brand-muted">
        Open a terminal in your project's repository and start a session:
      </p>
      <CodeBlock code="ololo start {created.slug ?? ''}" />
      <p class="mt-5 text-sm text-brand-muted">
        No ololo yet?
        <a href="#install" onclick={close} class="font-semibold text-brand-blue hover:opacity-70"
          >Install it</a
        >
        first. Its tasks and judges are on
        <a href="/projects/{created.id}" class="font-semibold text-brand-blue hover:opacity-70"
          >the project page</a
        >.
      </p>
    </div>
  {:else if draft && options}
    <div data-testid="own-project-suggestion">
      <p class="text-xs font-semibold uppercase tracking-wide text-brand-muted">
        Suggested project
      </p>
      <h3
        class="mt-1 font-heading text-[26px] font-bold leading-[1.23] text-brand-text"
        data-testid="own-project-name"
      >
        {draft.name}
      </h3>
      {#if draft.tasks.length > 0}
        <ol class="mt-5 space-y-3" data-testid="own-project-tasks">
          {#each draft.tasks as task, i (i)}
            <li class="flex gap-3">
              <span
                class="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-brand-light-blue text-xs font-semibold text-brand-blue"
                aria-hidden="true">{i + 1}</span
              >
              <span>
                <span class="block text-[15px] font-semibold text-brand-text">{task.title}</span>
                {#if task.description}
                  <span class="block text-sm text-brand-muted">{task.description}</span>
                {/if}
              </span>
            </li>
          {/each}
        </ol>
      {/if}
      {#if draft.note}
        <p class="mt-5 text-sm text-brand-muted" data-testid="own-project-note">{draft.note}</p>
      {/if}
      <p class="mt-5 text-sm text-brand-muted" data-testid="own-project-summary">
        {#if defaultJudges.length > 0}
          Reviewed by {defaultJudges.map((j) => j.name).join(", ")} ·
        {/if}
        {formatDuration(options.session.default_secs)} sessions · public: anyone can watch them
      </p>
      {#if createError}
        <p class="mt-4 text-sm text-red-500" data-testid="own-project-create-error">
          {createError}
        </p>
      {/if}
      <div class="mt-7 flex flex-col gap-3 sm:flex-row">
        <button
          type="button"
          onclick={start}
          disabled={creating}
          data-testid="own-project-start"
          class="rounded-md bg-[#0269fb] px-6 py-3 text-base font-semibold text-white transition-colors hover:bg-blue-700 disabled:cursor-not-allowed disabled:opacity-40"
        >
          {creating ? "Creating…" : "Start project"}
        </button>
        <button
          type="button"
          onclick={edit}
          disabled={creating}
          data-testid="own-project-edit"
          class="rounded-md border-2 border-[#0269fb] px-6 py-3 text-base font-semibold text-[#0269fb] transition-colors hover:bg-[#0269fb] hover:text-white disabled:cursor-not-allowed disabled:opacity-40"
        >
          Edit project
        </button>
      </div>
    </div>
  {/if}
</Modal>
