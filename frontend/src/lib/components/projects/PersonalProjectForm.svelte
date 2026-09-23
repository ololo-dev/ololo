<script lang="ts">
  // The personal-project form: the work described in the user's own words,
  // the navigation map it splits into, the judges who review each task and
  // the session length. Shared by create, edit (before the first session)
  // and duplicate. Posts to the page's default form action; the map and the
  // panel travel as JSON in hidden fields.
  import { enhance } from "$app/forms";
  import { untrack } from "svelte";
  import { suggestPersonalTasks, ApiError } from "$lib/api";
  import type { PersonalProjectOptions } from "$lib/api";
  import { formatDuration } from "$lib/format";

  interface Initial {
    name: string;
    description: string;
    tasks: { title: string; description: string }[];
    judges: string[];
    session_duration_secs: number;
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
  }

  let nextKey = 0;
  function row(title = "", description = ""): Row {
    nextKey += 1;
    return { key: nextKey, title, description, open: description.length > 0 };
  }

  const start = untrack(() => initial);
  let name = $state(start?.name ?? "");
  let description = $state(start?.description ?? "");
  let tasks = $state<Row[]>((start?.tasks ?? []).map((t) => row(t.title, t.description)));
  let judges = $state<string[]>(
    untrack(() =>
      start?.judges.length
        ? start.judges.filter((slug) => options.judges.some((j) => j.slug === slug))
        : options.judges.filter((j) => j.default).map((j) => j.slug),
    ),
  );
  let duration = $state(untrack(() => start?.session_duration_secs ?? options.session.default_secs));
  let submitting = $state(false);

  // "Suggest tasks": a draft map from the description, previewed before it
  // replaces anything the user typed.
  let suggesting = $state(false);
  let suggestError = $state<string | null>(null);
  let suggestion = $state<{ title: string; description: string }[] | null>(null);

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
      .map((t) => ({ title: t.title.trim(), description: t.description.trim() })),
  );
  const taskCount = $derived(Math.max(filledTasks.length, 1));
  // Each judge runs once per task, plus the session report at the end.
  const reviewCount = $derived(taskCount * judges.length + 1);
  const summaryText = $derived(
    `${taskCount} ${taskCount === 1 ? "task" : "tasks"} × ${judges.length} ` +
      `${judges.length === 1 ? "judge" : "judges"} → about ${reviewCount} judge reviews ` +
      "per session, including the session report.",
  );
  const canAddTask = $derived(tasks.length < options.limits.max_tasks);
  const descriptionTooLong = $derived(
    description.length > options.limits.max_description_chars,
  );
  const canSubmit = $derived(
    description.trim().length > 0 && judges.length > 0 && !descriptionTooLong && !submitting,
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
  function toggleJudge(slug: string) {
    if (judges.includes(slug)) {
      judges = judges.filter((s) => s !== slug);
    } else if (judges.length < options.limits.max_judges) {
      judges = [...judges, slug];
    }
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
      default:
        return "Something went wrong. Please try again.";
    }
  });
</script>

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
>
  <input type="hidden" name="tasks_json" value={JSON.stringify(filledTasks)} />
  <input type="hidden" name="judges_json" value={JSON.stringify(judges)} />
  <input type="hidden" name="session_duration_secs" value={duration} />

  <!-- The work -->
  <section class="rounded-[8px] bg-white px-6 py-6 md:px-[48px] md:py-[32px]">
    <h2 class="mb-1 font-heading text-[22px] font-bold text-brand-text">The work</h2>
    <p class="mb-5 text-sm text-brand-muted">
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
      rows="8"
      required
      placeholder="Add a CSV export to the reports page. It must honour the table's filters and include the totals row…"
      bind:value={description}
      data-testid="pp-description"
      class="w-full rounded-[8px] border-2 border-brand-border bg-white px-4 py-3 text-base text-brand-text placeholder:text-brand-muted focus:border-brand-blue focus:outline-none"
    ></textarea>
    <p class="mt-1 text-right text-xs {descriptionTooLong ? 'text-red-500' : 'text-brand-muted'}">
      {description.length} / {options.limits.max_description_chars}
    </p>
  </section>

  <!-- Navigation map -->
  <section class="mt-6 rounded-[8px] bg-white px-6 py-6 md:px-[48px] md:py-[32px]">
    <div class="mb-1 flex flex-wrap items-center justify-between gap-3">
      <h2 class="font-heading text-[22px] font-bold text-brand-text">Navigation map</h2>
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
    <p class="mb-5 text-sm text-brand-muted">
      The tasks the work splits into, in order. Each one is judged on its own and its code
      health is measured at its end. Leave it empty and the whole description is one task.
    </p>

    {#if suggestError}
      <p class="mb-4 text-sm text-red-500" data-testid="pp-suggest-error">{suggestError}</p>
    {/if}

    {#if suggestion}
      <div class="mb-5 rounded-[8px] border-2 border-dashed border-brand-blue bg-brand-light-blue p-4" data-testid="pp-suggestion">
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
      <p class="mb-4 rounded-[8px] bg-brand-light-blue px-4 py-3 text-sm text-brand-text" data-testid="pp-single-task">
        One task: the whole description.
      </p>
    {:else}
      <ol class="mb-4 flex flex-col gap-3" data-testid="pp-tasks">
        {#each tasks as task, i (task.key)}
          <li class="rounded-[8px] border-2 border-brand-border p-3">
            <div class="flex items-center gap-2">
              <span class="w-6 shrink-0 text-center text-sm font-semibold text-brand-muted">{i + 1}</span>
              <input
                type="text"
                aria-label="Task {i + 1} title"
                maxlength={options.limits.max_task_title_chars}
                placeholder="What this task delivers"
                bind:value={task.title}
                data-testid="pp-task-title"
                class="h-[40px] min-w-0 flex-1 rounded-[8px] border border-brand-border px-3 text-sm text-brand-text placeholder:text-brand-muted focus:border-brand-blue focus:outline-none"
              />
              <button
                type="button"
                onclick={() => (task.open = !task.open)}
                class="px-2 text-xs font-semibold text-brand-blue hover:opacity-70"
                aria-expanded={task.open}
              >
                {task.open ? "Hide details" : "Details"}
              </button>
              <button type="button" onclick={() => moveTask(task.key, -1)} disabled={i === 0}
                aria-label="Move task {i + 1} up"
                class="px-1 text-brand-muted hover:text-brand-text disabled:opacity-30">↑</button>
              <button type="button" onclick={() => moveTask(task.key, 1)} disabled={i === tasks.length - 1}
                aria-label="Move task {i + 1} down"
                class="px-1 text-brand-muted hover:text-brand-text disabled:opacity-30">↓</button>
              <button type="button" onclick={() => removeTask(task.key)}
                aria-label="Remove task {i + 1}"
                data-testid="pp-task-remove"
                class="px-1 text-brand-muted hover:text-red-500">✕</button>
            </div>
            {#if task.open}
              <textarea
                aria-label="Task {i + 1} details"
                rows="3"
                maxlength={options.limits.max_task_description_chars}
                placeholder="What is true when this task is done (optional)"
                bind:value={task.description}
                class="mt-2 w-full rounded-[8px] border border-brand-border px-3 py-2 text-sm text-brand-text placeholder:text-brand-muted focus:border-brand-blue focus:outline-none"
              ></textarea>
            {/if}
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

  <!-- Judges -->
  <section class="mt-6 rounded-[8px] bg-white px-6 py-6 md:px-[48px] md:py-[32px]">
    <h2 class="mb-1 font-heading text-[22px] font-bold text-brand-text">Judges</h2>
    <p class="mb-5 text-sm text-brand-muted">
      Who reviews every task — each judge reads the task's own changes, not the code it
      inherited. Up to {options.limits.max_judges}.
    </p>
    {#if options.judges.length === 0}
      <p class="text-sm text-red-500">No judges are set up on this instance yet.</p>
    {:else}
      <div class="grid grid-cols-1 gap-3 md:grid-cols-2" data-testid="pp-judges">
        {#each options.judges as judge (judge.slug)}
          {@const on = judges.includes(judge.slug)}
          <label
            class="flex cursor-pointer gap-3 rounded-[8px] border-2 p-3 transition-colors {on
              ? 'border-brand-blue bg-brand-light-blue'
              : 'border-brand-border hover:border-brand-blue'}"
          >
            <input
              type="checkbox"
              checked={on}
              onchange={() => toggleJudge(judge.slug)}
              disabled={!on && judges.length >= options.limits.max_judges}
              data-testid="pp-judge-{judge.slug}"
              class="mt-1 rounded border-brand-border"
            />
            <span>
              <span class="block text-sm font-semibold text-brand-text">{judge.name}</span>
              <span class="block text-xs leading-snug text-brand-muted">{judge.description}</span>
            </span>
          </label>
        {/each}
      </div>
    {/if}
  </section>

  <!-- Session -->
  <section class="mt-6 rounded-[8px] bg-white px-6 py-6 md:px-[48px] md:py-[32px]">
    <h2 class="mb-1 font-heading text-[22px] font-bold text-brand-text">Session</h2>
    <label for="pp-duration" class="mb-1 mt-3 block text-xs font-semibold text-brand-text">
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
    <p class="mt-4 text-sm text-brand-text" data-testid="pp-summary">{summaryText}</p>

    <div class="mt-5 rounded-[8px] bg-brand-light-blue px-4 py-3 text-sm leading-relaxed text-brand-text">
      <p class="mb-1 font-semibold">Your code, your project</p>
      <p>
        You run the session inside your own repository. It uploads what git tracks or would
        add — <code>.env</code> files, keys and credentials stay on your machine, and
        <code>.ololoignore</code> leaves out anything else — so the judges can read each task's
        changes. The project and its sessions are private to you and never count toward
        any ranking.
      </p>
    </div>
  </section>

  {#if errorText}
    <p class="mt-6 text-sm text-red-500" data-testid="pp-error">{errorText}</p>
  {/if}

  <div class="mt-8 flex items-center justify-end gap-4">
    <a href={cancelHref} class="text-sm font-semibold text-brand-blue hover:opacity-70">Cancel</a>
    <button
      type="submit"
      disabled={!canSubmit}
      data-testid="pp-submit"
      class="rounded-btn bg-brand-blue px-6 py-3 text-sm font-semibold text-white transition-opacity hover:opacity-80 disabled:cursor-not-allowed disabled:opacity-40"
    >
      {submitting ? "Saving…" : submitLabel}
    </button>
  </div>
</form>
