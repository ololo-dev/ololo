<script lang="ts">
  // The personal-project form: the work described in the user's own words,
  // the navigation map it splits into — each task reviewed by the default
  // judges or by judges of its own — the session length and who sees it,
  // next to a summary that keeps the cost of a session in view. Shared by
  // create, edit (before the first session) and duplicate. Posts to the
  // page's default form action; the map and the panels travel as JSON in
  // hidden fields.
  import { enhance } from "$app/forms";
  import { untrack } from "svelte";
  import { suggestPersonalTasks, ApiError } from "$lib/api";
  import type { PersonalProjectOptions, PersonalJudgeOption } from "$lib/api";
  import { formatDuration } from "$lib/format";

  interface Initial {
    name: string;
    description: string;
    /** `judges`: the task's own panel; absent = the default panel. */
    tasks: { title: string; description: string; judges?: string[] }[];
    judges: string[];
    session_duration_secs: number;
    /** Absent: public. */
    public?: boolean;
  }

  interface FormError {
    error: string;
    field?: string | null;
    detail?: string | null;
  }

  let {
    options,
    initial = null,
    submitLabel,
    cancelHref,
    error = null,
  }: {
    options: PersonalProjectOptions;
    initial?: Initial | null;
    submitLabel: string;
    cancelHref: string;
    error?: FormError | null;
  } = $props();

  interface Row {
    key: number;
    title: string;
    description: string;
    open: boolean;
    /** The task's own panel; null = the default panel. */
    judges: string[] | null;
  }

  const offered = untrack(() => new Set(options.judges.map((j) => j.slug)));

  let nextKey = 0;
  function row(title = "", description = "", judges: string[] | null = null): Row {
    nextKey += 1;
    return {
      key: nextKey,
      title,
      description,
      open: description.length > 0,
      judges: judges && judges.filter((slug) => offered.has(slug)),
    };
  }

  const start = untrack(() => initial);
  let name = $state(start?.name ?? "");
  let description = $state(start?.description ?? "");
  let tasks = $state<Row[]>(
    (start?.tasks ?? []).map((t) => row(t.title, t.description, t.judges ?? null)),
  );
  let judges = $state<string[]>(
    untrack(() =>
      start?.judges.length
        ? start.judges.filter((slug) => offered.has(slug))
        : options.judges.filter((j) => j.default).map((j) => j.slug),
    ),
  );
  let duration = $state(untrack(() => start?.session_duration_secs ?? options.session.default_secs));
  let isPublic = $state(start?.public ?? true);
  // Keeping a private project private is always possible; making one
  // private takes Premium where plans are on.
  const privateSelectable = untrack(() => options.private_allowed || start?.public === false);
  let submitting = $state(false);

  // "Suggest tasks": a draft map from the description, previewed before it
  // replaces anything the user typed.
  let suggesting = $state(false);
  let suggestError = $state<string | null>(null);
  let suggestion = $state<{ title: string; description: string }[] | null>(null);

  const judgeNames = $derived(new Map(options.judges.map((j) => [j.slug, j.name])));

  const durationChoices = $derived(
    [30, 60, 90, 120, 180, 240, 360, 480]
      .map((m) => m * 60)
      .filter((s) => s >= options.session.min_secs && s <= options.session.max_secs)
      .concat(duration)
      .filter((s, i, all) => all.indexOf(s) === i)
      .sort((a, b) => a - b),
  );

  const filledTasks = $derived(
    tasks
      .filter((t) => t.title.trim().length > 0)
      .map((t) => ({
        title: t.title.trim(),
        description: t.description.trim(),
        ...(t.judges ? { judges: t.judges } : {}),
      })),
  );
  const ownPanels = $derived(filledTasks.filter((t) => t.judges).length);
  // Every judge of a task's panel reviews it once; the session report is
  // one more review at the end.
  const reviewCount = $derived(
    (filledTasks.length === 0
      ? judges.length
      : filledTasks.reduce((sum, t) => sum + (t.judges ?? judges).length, 0)) + 1,
  );
  const canAddTask = $derived(tasks.length < options.limits.max_tasks);
  const descriptionTooLong = $derived(
    description.length > options.limits.max_description_chars,
  );
  // The positions of the tasks that chose their own judges and then
  // unchecked them all.
  const emptyPanels = $derived(
    tasks.flatMap((t, i) => (t.judges !== null && t.judges.length === 0 ? [i + 1] : [])),
  );
  const canSubmit = $derived(
    description.trim().length > 0 &&
      judges.length > 0 &&
      emptyPanels.length === 0 &&
      !descriptionTooLong &&
      !submitting,
  );

  function addTask() {
    if (canAddTask) tasks = [...tasks, row()];
  }
  function removeTask(key: number) {
    tasks = tasks.filter((t) => t.key !== key);
  }
  function moveTask(key: number, by: number) {
    const i = tasks.findIndex((t) => t.key === key);
    const j = i + by;
    if (i < 0 || j < 0 || j >= tasks.length) return;
    const next = [...tasks];
    [next[i], next[j]] = [next[j], next[i]];
    tasks = next;
  }
  function toggled(panel: string[], slug: string): string[] {
    if (panel.includes(slug)) return panel.filter((s) => s !== slug);
    if (panel.length >= options.limits.max_judges) return panel;
    return [...panel, slug];
  }
  function toggleJudge(slug: string) {
    judges = toggled(judges, slug);
  }
  function toggleTaskJudge(task: Row, slug: string) {
    task.judges = toggled(task.judges ?? [], slug);
  }

  async function suggest() {
    suggestError = null;
    suggesting = true;
    try {
      const drafted = await suggestPersonalTasks(description, name.trim() || undefined);
      suggestion = drafted.map((t) => ({ title: t.title, description: t.description ?? "" }));
    } catch (e) {
      suggestError =
        e instanceof ApiError && e.code === "rate_limited"
          ? "Too many suggestions in a row — try again in a few minutes."
          : e instanceof ApiError && e.code === "no_model_configured"
            ? "Suggestions are not available on this instance."
            : "Could not draft tasks right now. Write them yourself, or try again.";
    } finally {
      suggesting = false;
    }
  }
  function acceptSuggestion() {
    if (!suggestion) return;
    tasks = suggestion.slice(0, options.limits.max_tasks).map((t) => row(t.title, t.description));
    suggestion = null;
  }

  function fieldLabel(field: string | null | undefined): string {
    switch (field) {
      case "description":
        return "Description";
      case "name":
        return "Name";
      case "tasks":
        return "Navigation map";
      case "judges":
        return "Judges";
      case "session_duration_secs":
        return "Session length";
      default:
        return "Project";
    }
  }

  const errorText = $derived.by(() => {
    if (!error) return null;
    switch (error.error) {
      case "invalid_personal_project":
        return `${fieldLabel(error.field)}: ${error.detail ?? "invalid"}`;
      case "project_frozen":
        return "This project has been played, so its tasks are frozen. Duplicate it to change them.";
      case "no_judges_available":
        return "No judges are set up on this instance yet.";
      case "creation_restricted":
        return "Project creation is not enabled for your account.";
      case "premium_required":
        return "Private projects are part of Premium — this one can be public.";
      default:
        return "Something went wrong. Please try again.";
    }
  });
</script>

{#snippet heading(step: number, title: string)}
  <h2 class="flex items-center gap-3 font-heading text-[22px] font-bold text-brand-text">
    <span
      class="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-brand-blue text-[13px] text-white"
      aria-hidden="true">{step}</span
    >
    {title}
  </h2>
{/snippet}

{#snippet judgeCard(judge: PersonalJudgeOption)}
  {@const on = judges.includes(judge.slug)}
  <label
    class="flex cursor-pointer gap-3 rounded-[8px] border-2 p-3 transition-colors {on
      ? 'border-brand-blue bg-brand-light-blue'
      : 'border-brand-border hover:border-brand-check-blue'}"
  >
    <input
      type="checkbox"
      checked={on}
      onchange={() => toggleJudge(judge.slug)}
      disabled={!on && judges.length >= options.limits.max_judges}
      data-testid="pp-judge-{judge.slug}"
      class="mt-1 shrink-0 rounded border-brand-border"
    />
    <span class="min-w-0">
      <span class="block text-sm font-semibold text-brand-text">{judge.name}</span>
      <span class="line-clamp-2 text-xs leading-snug text-brand-muted" title={judge.description}
        >{judge.description}</span
      >
      {#if judge.criteria.length > 0}
        <span class="mt-2 flex flex-wrap gap-1">
          {#each judge.criteria as criterion (criterion)}
            <span class="rounded bg-brand-tag-bg px-1.5 py-px text-[11px] text-brand-deep-blue"
              >{criterion}</span
            >
          {/each}
        </span>
      {/if}
    </span>
  </label>
{/snippet}

<form
  method="POST"
  use:enhance={() => {
    submitting = true;
    return async ({ update }) => {
      await update({ reset: false });
      submitting = false;
    };
  }}
  data-testid="personal-project-form"
  class="grid grid-cols-1 items-start gap-6 lg:grid-cols-[minmax(0,1fr)_300px]"
>
  <input type="hidden" name="tasks_json" value={JSON.stringify(filledTasks)} />
  <input type="hidden" name="judges_json" value={JSON.stringify(judges)} />
  <input type="hidden" name="session_duration_secs" value={duration} />

  <div class="flex min-w-0 flex-col gap-6">
    <!-- 1. The work -->
    <section class="rounded-[8px] bg-white px-6 py-6 md:px-[40px] md:py-[32px]">
      {@render heading(1, "The work")}
      <p class="mb-5 mt-1 text-sm text-brand-muted">
        What should your agent do in your repository? Write it the way you would brief a
        colleague — the judges read the same words.
      </p>

      <label for="pp-name" class="mb-1 block text-xs font-semibold text-brand-text">
        Name <span class="font-normal text-brand-muted">(optional — taken from the description)</span>
      </label>
      <input
        id="pp-name"
        name="name"
        type="text"
        maxlength={options.limits.max_name_chars}
        placeholder="e.g. CSV export for the reports page"
        bind:value={name}
        data-testid="pp-name"
        class="mb-4 h-[48px] w-full rounded-[8px] border-2 border-brand-border bg-white px-4 text-base text-brand-text placeholder:text-brand-muted focus:border-brand-blue focus:outline-none"
      />

      <label for="pp-description" class="mb-1 block text-xs font-semibold text-brand-text">
        Description
      </label>
      <textarea
        id="pp-description"
        name="description"
        rows="7"
        required
        placeholder="Add a CSV export to the reports page. It must honour the table's filters and include the totals row…"
        bind:value={description}
        data-testid="pp-description"
        class="w-full rounded-[8px] border-2 border-brand-border bg-white px-4 py-3 text-base text-brand-text placeholder:text-brand-muted focus:border-brand-blue focus:outline-none"
      ></textarea>
      <p class="mt-1 flex justify-between gap-4 text-xs text-brand-muted">
        <span>Say what “done” means: the behaviour you expect, the cases that matter.</span>
        <span class="shrink-0 {descriptionTooLong ? 'text-red-500' : ''}">
          {description.length} / {options.limits.max_description_chars}
        </span>
      </p>
    </section>

    <!-- 2. Navigation map -->
    <section class="rounded-[8px] bg-white px-6 py-6 md:px-[40px] md:py-[32px]">
      <div class="flex flex-wrap items-center justify-between gap-3">
        {@render heading(2, "Navigation map")}
        {#if options.suggest_available}
          <button
            type="button"
            onclick={suggest}
            disabled={suggesting || description.trim().length === 0}
            data-testid="pp-suggest"
            class="rounded-[8px] border border-brand-blue px-4 py-2 text-sm font-semibold text-brand-blue transition-opacity hover:opacity-80 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {suggesting ? "Drafting…" : "Suggest tasks"}
          </button>
        {/if}
      </div>
      <p class="mb-5 mt-1 text-sm text-brand-muted">
        The tasks the work splits into, in order. Each one is reviewed on its own and its code
        health is measured at its end. Leave it empty and the whole description is one task.
      </p>

      {#if suggestError}
        <p class="mb-4 text-sm text-red-500" data-testid="pp-suggest-error">{suggestError}</p>
      {/if}

      {#if suggestion}
        <div
          class="mb-5 rounded-[8px] border-2 border-dashed border-brand-blue bg-brand-light-blue p-4"
          data-testid="pp-suggestion"
        >
          <p class="mb-2 text-sm font-semibold text-brand-text">Suggested map</p>
          <ol class="mb-3 list-decimal pl-5 text-sm text-brand-text">
            {#each suggestion as s, i (i)}
              <li class="mb-1">
                <span class="font-semibold">{s.title}</span>
                {#if s.description}<span class="text-brand-muted"> — {s.description}</span>{/if}
              </li>
            {/each}
          </ol>
          <div class="flex gap-3">
            <button
              type="button"
              onclick={acceptSuggestion}
              data-testid="pp-suggestion-accept"
              class="rounded-[8px] bg-brand-blue px-4 py-2 text-sm font-semibold text-white hover:opacity-80"
            >
              {tasks.length > 0 ? "Replace my map" : "Use these tasks"}
            </button>
            <button
              type="button"
              onclick={() => (suggestion = null)}
              class="px-2 py-2 text-sm font-semibold text-brand-blue hover:opacity-70"
            >
              Discard
            </button>
          </div>
        </div>
      {/if}

      {#if tasks.length === 0}
        <p
          class="mb-4 rounded-[8px] bg-brand-light-blue px-4 py-3 text-sm text-brand-text"
          data-testid="pp-single-task"
        >
          One task: the whole description, reviewed by the default judges.
        </p>
      {:else}
        <ol class="mb-4 flex flex-col gap-3" data-testid="pp-tasks">
          {#each tasks as task, i (task.key)}
            <li class="rounded-[8px] border-2 border-brand-border p-3" data-testid="pp-task">
              <!-- On a phone the title takes a line of its own, under the
                   number and the controls. -->
              <div class="flex flex-wrap items-center gap-2 sm:flex-nowrap">
                <span
                  class="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-brand-tag-bg text-xs font-bold text-brand-blue"
                  >{i + 1}</span
                >
                <input
                  type="text"
                  aria-label="Task {i + 1} title"
                  maxlength={options.limits.max_task_title_chars}
                  placeholder="What this task delivers"
                  bind:value={task.title}
                  data-testid="pp-task-title"
                  class="order-last h-[40px] min-w-0 basis-full rounded-[8px] border border-brand-border px-3 text-sm text-brand-text placeholder:text-brand-muted focus:border-brand-blue focus:outline-none sm:order-none sm:basis-auto sm:flex-1"
                />
                <div class="ml-auto flex shrink-0">
                  <button
                    type="button"
                    onclick={() => moveTask(task.key, -1)}
                    disabled={i === 0}
                    aria-label="Move task {i + 1} up"
                    class="h-8 w-8 rounded text-brand-muted hover:bg-brand-light-blue hover:text-brand-text disabled:opacity-30"
                    >↑</button
                  >
                  <button
                    type="button"
                    onclick={() => moveTask(task.key, 1)}
                    disabled={i === tasks.length - 1}
                    aria-label="Move task {i + 1} down"
                    class="h-8 w-8 rounded text-brand-muted hover:bg-brand-light-blue hover:text-brand-text disabled:opacity-30"
                    >↓</button
                  >
                  <button
                    type="button"
                    onclick={() => removeTask(task.key)}
                    aria-label="Remove task {i + 1}"
                    data-testid="pp-task-remove"
                    class="h-8 w-8 rounded text-brand-muted hover:bg-red-50 hover:text-red-500"
                    >✕</button
                  >
                </div>
              </div>

              <div class="mt-2 flex flex-col gap-2 sm:pl-8">
                {#if task.open}
                  <textarea
                    aria-label="Task {i + 1} details"
                    rows="2"
                    maxlength={options.limits.max_task_description_chars}
                    placeholder="What is true when this task is done (optional)"
                    bind:value={task.description}
                    class="w-full rounded-[8px] border border-brand-border px-3 py-2 text-sm text-brand-text placeholder:text-brand-muted focus:border-brand-blue focus:outline-none"
                  ></textarea>
                {/if}

                {#if task.judges === null}
                  <div class="flex flex-wrap items-center gap-x-3 gap-y-1.5 text-xs">
                    {#if !task.open}
                      <button
                        type="button"
                        onclick={() => (task.open = true)}
                        class="font-semibold text-brand-blue hover:opacity-70">+ Details</button
                      >
                    {/if}
                    <span class="flex flex-wrap items-center gap-1" data-testid="pp-task-judges">
                      <span class="text-brand-muted">Default judges:</span>
                      {#each judges as slug (slug)}
                        <span class="rounded-full bg-brand-light-blue px-2 py-0.5 text-brand-deep-blue"
                          >{judgeNames.get(slug) ?? slug}</span
                        >
                      {:else}
                        <span class="text-red-500">none yet</span>
                      {/each}
                    </span>
                    <button
                      type="button"
                      onclick={() => (task.judges = [...judges])}
                      data-testid="pp-task-customize"
                      class="font-semibold text-brand-blue hover:opacity-70">Choose judges</button
                    >
                  </div>
                {:else}
                  {#if !task.open}
                    <button
                      type="button"
                      onclick={() => (task.open = true)}
                      class="self-start text-xs font-semibold text-brand-blue hover:opacity-70"
                      >+ Details</button
                    >
                  {/if}
                  <div
                    class="rounded-[8px] bg-brand-light-blue p-3"
                    role="group"
                    aria-label="Judges of task {i + 1}"
                    data-testid="pp-task-panel"
                  >
                    <div class="mb-2 flex flex-wrap items-center justify-between gap-2 text-xs">
                      <span class="font-semibold text-brand-text">This task's judges</span>
                      <button
                        type="button"
                        onclick={() => (task.judges = null)}
                        data-testid="pp-task-inherit"
                        class="font-semibold text-brand-blue hover:opacity-70"
                        >Use the default judges</button
                      >
                    </div>
                    <div class="flex flex-wrap gap-1.5">
                      {#each options.judges as judge (judge.slug)}
                        {@const on = task.judges.includes(judge.slug)}
                        <button
                          type="button"
                          aria-pressed={on}
                          onclick={() => toggleTaskJudge(task, judge.slug)}
                          disabled={!on && task.judges.length >= options.limits.max_judges}
                          title={judge.description}
                          data-testid="pp-task-judge-{judge.slug}"
                          class="rounded-full border px-3 py-1 text-xs font-semibold transition-colors disabled:cursor-not-allowed disabled:opacity-40 {on
                            ? 'border-brand-blue bg-brand-blue text-white'
                            : 'border-brand-border bg-white text-brand-text hover:border-brand-blue'}"
                          >{judge.name}</button
                        >
                      {/each}
                    </div>
                    {#if task.judges.length === 0}
                      <p class="mt-2 text-xs text-red-500">Pick at least one judge for this task.</p>
                    {/if}
                  </div>
                {/if}
              </div>
            </li>
          {/each}
        </ol>
      {/if}

      <button
        type="button"
        onclick={addTask}
        disabled={!canAddTask}
        data-testid="pp-add-task"
        class="rounded-[8px] border border-dashed border-brand-blue px-4 py-2 text-sm font-semibold text-brand-blue transition-colors hover:bg-brand-light-blue disabled:cursor-not-allowed disabled:opacity-40"
      >
        + Add task
      </button>
      {#if !canAddTask}
        <span class="ml-2 text-xs text-brand-muted">at most {options.limits.max_tasks} tasks</span>
      {/if}
    </section>

    <!-- 3. Default judges -->
    <section class="rounded-[8px] bg-white px-6 py-6 md:px-[40px] md:py-[32px]">
      {@render heading(3, "Judges")}
      <p class="mb-5 mt-1 text-sm text-brand-muted">
        The default panel: it reviews every task that does not choose its own. Each judge reads
        the task's own changes, not the code it inherited. Up to {options.limits.max_judges}.
      </p>
      {#if options.judges.length === 0}
        <p class="text-sm text-red-500">No judges are set up on this instance yet.</p>
      {:else}
        <div class="grid grid-cols-1 gap-3 md:grid-cols-2" data-testid="pp-judges">
          {#each options.judges as judge (judge.slug)}
            {@render judgeCard(judge)}
          {/each}
        </div>
      {/if}
    </section>

    <!-- 4. Session -->
    <section class="rounded-[8px] bg-white px-6 py-6 md:px-[40px] md:py-[32px]">
      {@render heading(4, "Session")}
      <label for="pp-duration" class="mb-1 mt-4 block text-xs font-semibold text-brand-text">
        Length
      </label>
      <select
        id="pp-duration"
        bind:value={duration}
        data-testid="pp-duration"
        class="h-[48px] w-full max-w-[240px] rounded-[8px] border-2 border-brand-border bg-white px-4 text-base text-brand-text focus:border-brand-blue focus:outline-none"
      >
        {#each durationChoices as secs (secs)}
          <option value={secs}>{formatDuration(secs)}</option>
        {/each}
      </select>
      <p class="mt-2 text-xs text-brand-muted">
        One clock for the whole map: the tasks run one after another, and the session ends
        when the last one is done or the time runs out. Give a session's join code to a
        teammate and they play it too — in their own copy of the repository, judged on their
        own work.
      </p>
    </section>

    <!-- 5. Who sees it -->
    <section class="rounded-[8px] bg-white px-6 py-6 md:px-[40px] md:py-[32px]">
      {@render heading(5, "Who sees it")}
      <p class="mb-5 mt-1 text-sm text-brand-muted">
        Either way it stays out of the project catalog and never counts toward any ranking.
      </p>
      <div class="grid grid-cols-1 gap-3 md:grid-cols-2" data-testid="pp-visibility">
        <label
          class="flex cursor-pointer gap-3 rounded-[8px] border-2 p-3 transition-colors {isPublic
            ? 'border-brand-blue bg-brand-light-blue'
            : 'border-brand-border hover:border-brand-check-blue'}"
        >
          <input
            type="radio"
            name="public"
            value="true"
            checked={isPublic}
            onchange={() => (isPublic = true)}
            data-testid="pp-public"
            class="mt-1 shrink-0"
          />
          <span class="min-w-0">
            <span class="block text-sm font-semibold text-brand-text">Public</span>
            <span class="block text-xs leading-snug text-brand-muted">
              Listed on your profile. Anyone can open it and watch its sessions — the code each
              task changes included — and a live one shows on the landing page.
            </span>
          </span>
        </label>
        <label
          class="flex gap-3 rounded-[8px] border-2 p-3 transition-colors {!isPublic
            ? 'border-brand-blue bg-brand-light-blue'
            : 'border-brand-border'} {privateSelectable
            ? 'cursor-pointer hover:border-brand-check-blue'
            : 'cursor-not-allowed'}"
        >
          <input
            type="radio"
            name="public"
            value="false"
            checked={!isPublic}
            disabled={!privateSelectable}
            onchange={() => (isPublic = false)}
            data-testid="pp-private"
            class="mt-1 shrink-0"
          />
          <span class="min-w-0">
            <span class="flex items-center gap-2 text-sm font-semibold text-brand-text">
              Private
              {#if !privateSelectable}
                <span
                  class="rounded-full bg-amber-50 px-2 py-px text-[11px] font-semibold text-amber-700"
                  >Premium</span
                >
              {/if}
            </span>
            <span class="block text-xs leading-snug text-brand-muted">
              Only you see the project, and a session only whoever you give its join code.
            </span>
            {#if !privateSelectable}
              <a
                href="/pricing"
                data-testid="pp-private-upsell"
                class="mt-1 inline-block text-xs font-semibold text-brand-blue hover:opacity-70"
                >Private projects come with Premium →</a
              >
            {/if}
          </span>
        </label>
      </div>
    </section>
  </div>

  <!-- What a session of this project will be, and the way out -->
  <aside
    class="rounded-[8px] bg-white px-6 py-6 lg:sticky lg:top-6"
    aria-label="Session summary"
    data-testid="pp-summary-card"
  >
    <h2 class="mb-4 font-heading text-[18px] font-bold text-brand-text">Every session</h2>
    <dl class="flex flex-col gap-2.5 text-sm">
      <div class="flex justify-between gap-3">
        <dt class="text-brand-muted">Tasks</dt>
        <dd class="text-right font-semibold text-brand-text" data-testid="pp-task-count">
          {filledTasks.length === 0 ? "1 — the whole work" : filledTasks.length}
        </dd>
      </div>
      <div class="flex justify-between gap-3">
        <dt class="text-brand-muted">Default judges</dt>
        <dd class="text-right font-semibold text-brand-text">{judges.length}</dd>
      </div>
      {#if ownPanels > 0}
        <div class="flex justify-between gap-3">
          <dt class="text-brand-muted">Tasks with their own judges</dt>
          <dd class="text-right font-semibold text-brand-text" data-testid="pp-own-panels">
            {ownPanels}
          </dd>
        </div>
      {/if}
      <div class="flex justify-between gap-3">
        <dt class="text-brand-muted">Length</dt>
        <dd class="text-right font-semibold text-brand-text">{formatDuration(duration)}</dd>
      </div>
      <div class="flex justify-between gap-3">
        <dt class="text-brand-muted">Who sees it</dt>
        <dd class="text-right font-semibold text-brand-text" data-testid="pp-visibility-summary">
          {isPublic ? "Anyone" : "Only you"}
        </dd>
      </div>
    </dl>
    <p class="mt-4 border-t border-brand-border pt-4 text-sm text-brand-text" data-testid="pp-summary">
      About <span class="font-semibold">{reviewCount} judge reviews</span> per session:
      {reviewCount - 1} for {filledTasks.length > 1 ? "the tasks" : "the task"} and 1 for the session
      report. Judges award up to {options.task_points} points a task.
    </p>

    <div class="mt-4 rounded-[8px] bg-brand-light-blue px-4 py-3 text-xs leading-relaxed text-brand-text">
      <p class="mb-1 font-semibold">Your code, your project</p>
      <p>
        You run it in your own repository. It uploads what git tracks or would add —
        <code>.env</code> files, keys and credentials stay on your machine, and
        <code>.ololoignore</code> leaves out anything else.
        {isPublic
          ? "Public: its sessions, and that code, are anyone's to watch."
          : "Private: only you, and whoever you give a session's join code, see it."}
      </p>
    </div>

    {#if emptyPanels.length > 0}
      <p class="mt-4 text-sm text-red-500" data-testid="pp-empty-panels">
        {emptyPanels.length === 1
          ? `Task ${emptyPanels[0]} has no judges.`
          : `Tasks ${emptyPanels.join(", ")} have no judges.`}
      </p>
    {/if}
    {#if errorText}
      <p class="mt-4 text-sm text-red-500" data-testid="pp-error">{errorText}</p>
    {/if}

    <button
      type="submit"
      disabled={!canSubmit}
      data-testid="pp-submit"
      class="mt-5 w-full rounded-btn bg-brand-blue px-6 py-3 text-sm font-semibold text-white transition-opacity hover:opacity-80 disabled:cursor-not-allowed disabled:opacity-40"
    >
      {submitting ? "Saving…" : submitLabel}
    </button>
    <a
      href={cancelHref}
      class="mt-3 block text-center text-sm font-semibold text-brand-blue hover:opacity-70">Cancel</a
    >
  </aside>
</form>
