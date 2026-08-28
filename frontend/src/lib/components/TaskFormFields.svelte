<script lang="ts">
  import MarkdownField from "$lib/components/MarkdownField.svelte";
  import type { Task } from "$lib/api";

  interface Props {
    mode: "add" | "edit";
    selectedTask: Task | null;
    addTitle: string;
    description: string;
    commandTemplate: string;
    tags: string[];
    tagInput: string;
    pointsValue: string;
    pointsFail: string;
    pointsNoResponse: string;
    pointsCompletionBonus: string;
    intervalsDeadline: string;
    intervalsMin: string;
    intervalsIncrement: string;
    intervalsMax: string;
    beautifyLoading: boolean;
    beautifyError: string | null;
    generateLoading: boolean;
    generateError: string | null;
    beautifyFormId: string;
    generateFormId: string;
    answerTemplate: string;
    fixtures: string;
  }

  let {
    mode,
    selectedTask,
    addTitle = $bindable(),
    description = $bindable(),
    commandTemplate = $bindable(),
    tags = $bindable(),
    tagInput = $bindable(),
    pointsValue = $bindable(),
    pointsFail = $bindable(),
    pointsNoResponse = $bindable(),
    pointsCompletionBonus = $bindable(),
    intervalsDeadline = $bindable(),
    intervalsMin = $bindable(),
    intervalsIncrement = $bindable(),
    intervalsMax = $bindable(),
    beautifyLoading = $bindable(),
    beautifyError = $bindable(),
    generateLoading = $bindable(),
    generateError = $bindable(),
    beautifyFormId,
    generateFormId,
    answerTemplate,
    fixtures,
  }: Props = $props();

  const pf = $derived(mode === "add" ? "add" : "edit-task");

  function confirmTag() {
    const trimmed = tagInput.trim().replace(/[, ]+$/, "").trim();
    if (!trimmed || tags.includes(trimmed) || tags.length >= 20 || trimmed.length > 128) {
      tagInput = "";
      return;
    }
    tags = [...tags, trimmed];
    tagInput = "";
  }

  function removeTag(tag: string) {
    tags = tags.filter((t) => t !== tag);
  }
</script>

<h3 class="mb-4 text-base font-semibold text-brand-text">Task settings</h3>
<div class="space-y-4">
  <div>
    <label for="{pf}-task-title" class="mb-1 block text-xs font-semibold text-brand-text">
      Title <span class="text-brand-red">*</span>
    </label>
    {#if mode === "add"}
      <input
        id="{pf}-task-title"
        name="title"
        type="text"
        required
        maxlength="200"
        placeholder="Enter the title"
        oninput={(e) => { addTitle = (e.target as HTMLInputElement).value; }}
        class="h-[48px] w-full rounded-[8px] border-2 border-brand-border bg-white px-4
               text-base text-brand-text placeholder:text-brand-muted
               focus:border-brand-blue focus:outline-none"
        data-testid="task-title"
      />
    {:else}
      <input
        id="{pf}-task-title"
        name="title"
        type="text"
        required
        maxlength="200"
        value={selectedTask?.title}
        placeholder="Enter the title"
        class="h-[48px] w-full rounded-[8px] border-2 border-brand-border bg-white px-4
               text-base text-brand-text placeholder:text-brand-muted
               focus:border-brand-blue focus:outline-none"
        data-testid="task-title"
      />
    {/if}
  </div>

  {#if mode === "add"}
    <div>
      <label for="{pf}-task-cmd" class="mb-1 block text-xs font-semibold text-brand-text">
        Command template <span class="text-brand-red">*</span>
      </label>
      <div oninput={(e) => { commandTemplate = (e.target as HTMLTextAreaElement).value; }}>
        <MarkdownField
          name="command_template"
          value={commandTemplate}
          placeholder={"```bash\ncargo test {task}\n```\n\n## Checks\n- exit: 0\n- output: test result: ok"}
          data-testid="command-template"
        />
      </div>
    </div>
  {/if}

  <div>
    <p class="mb-1 text-xs font-semibold text-brand-text">Description</p>
    <div oninput={(e) => { description = (e.target as HTMLTextAreaElement).value; }}>
      <MarkdownField name="description" value={description} placeholder="Optional description" />
    </div>
    <div class="mt-1 flex gap-2">
      <button
        type="button"
        class="rounded border border-brand-border px-2 py-0.5 text-xs text-brand-text transition-colors hover:bg-brand-light-blue disabled:cursor-not-allowed disabled:opacity-50"
        disabled={beautifyLoading}
        onclick={() => (document.getElementById(beautifyFormId) as HTMLFormElement)?.requestSubmit()}
      >
        {beautifyLoading ? "Beautifying…" : "Beautify"}
      </button>
      <button
        type="button"
        class="rounded border border-brand-border px-2 py-0.5 text-xs text-brand-text transition-colors hover:bg-brand-light-blue disabled:cursor-not-allowed disabled:opacity-50"
        disabled={generateLoading}
        onclick={() => (document.getElementById(generateFormId) as HTMLFormElement)?.requestSubmit()}
      >
        {generateLoading ? "Generating…" : "Generate Tests"}
      </button>
    </div>
    {#if beautifyError}<p class="mt-1 text-xs text-brand-red">{beautifyError}</p>{/if}
    {#if generateError}<p class="mt-1 text-xs text-brand-red">{generateError}</p>{/if}
  </div>

  {#if mode === "edit"}
    <div>
      <label for="{pf}-task-cmd" class="mb-1 block text-xs font-semibold text-brand-text">
        Command template <span class="text-brand-red">*</span>
      </label>
      <div oninput={(e) => { commandTemplate = (e.target as HTMLTextAreaElement).value; }}>
        <MarkdownField
          name="command_template"
          value={commandTemplate}
          placeholder={"```bash\ncargo test {task}\n```\n\n## Checks\n- exit: 0\n- output: test result: ok"}
          data-testid="command-template"
        />
      </div>
    </div>
  {/if}

  <!-- Tags -->
  <div>
    <p class="mb-1 text-xs font-semibold text-brand-text">
      Tags <span class="font-normal text-brand-muted">(optional, max 20)</span>
    </p>
    {#if tags.length > 0}
      <div class="mb-1.5 flex flex-wrap gap-1" data-testid="drawer-task-tags">
        {#each tags as tag}
          <span class="inline-flex items-center gap-0.5 rounded-[4px] bg-brand-light-blue px-2 py-0.5 text-xs text-brand-text">
            {tag}
            <button
              type="button"
              onclick={() => removeTag(tag)}
              class="ml-0.5 leading-none text-brand-muted hover:text-brand-red"
              aria-label="Remove tag {tag}"
            >×</button>
          </span>
        {/each}
      </div>
    {/if}
    <input
      type="text"
      value={tagInput}
      placeholder="Add tag (comma, space or Enter to confirm)…"
      maxlength="129"
      data-testid="drawer-tag-input"
      class="h-[36px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3
             text-sm text-brand-text placeholder:text-brand-muted
             focus:border-brand-blue focus:outline-none"
      oninput={(e) => {
        const v = (e.target as HTMLInputElement).value;
        if (v.endsWith(",") || v.endsWith(" ")) {
          tagInput = v.slice(0, -1).trim();
          confirmTag();
        } else {
          tagInput = v;
        }
      }}
      onkeydown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          confirmTag();
        }
      }}
    />
    <input type="hidden" name="answer_template" value={answerTemplate} />
    <input type="hidden" name="fixtures" value={fixtures} />
    <input type="hidden" name="tags_json" value={JSON.stringify(tags)} data-testid={mode === "add" ? "add-task-tags-json" : undefined} />
  </div>
</div>

<!-- Points overrides (blank = inherit/preserve) -->
<div class="mt-5 border-t border-brand-border pt-4">
  <h3 class="mb-3 text-base font-semibold text-brand-text">
    Points <span class="text-xs font-normal text-brand-muted">(blank = {mode === "add" ? "inherit project default" : "preserve current value"})</span>
  </h3>
  <div class="grid grid-cols-2 gap-3">
    <div>
      <label for="{pf}-points-value" class="mb-1 block text-xs font-semibold text-brand-text">Point value</label>
      <input id="{pf}-points-value" name="points_value" type="number" value={pointsValue}
        oninput={(e) => (pointsValue = (e.target as HTMLInputElement).value)}
        class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3 text-base text-brand-text focus:border-brand-blue focus:outline-none" />
    </div>
    <div>
      <label for="{pf}-points-fail" class="mb-1 block text-xs font-semibold text-brand-text">Fail points</label>
      <input id="{pf}-points-fail" name="points_fail" type="number" value={pointsFail}
        oninput={(e) => (pointsFail = (e.target as HTMLInputElement).value)}
        class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3 text-base text-brand-text focus:border-brand-blue focus:outline-none" />
    </div>
    <div>
      <label for="{pf}-points-no-response" class="mb-1 block text-xs font-semibold text-brand-text">No-response points</label>
      <input id="{pf}-points-no-response" name="points_no_response" type="number" value={pointsNoResponse}
        oninput={(e) => (pointsNoResponse = (e.target as HTMLInputElement).value)}
        class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3 text-base text-brand-text focus:border-brand-blue focus:outline-none" />
    </div>
    <div>
      <label for="{pf}-points-completion-bonus" class="mb-1 block text-xs font-semibold text-brand-text">Completion bonus</label>
      <input id="{pf}-points-completion-bonus" name="points_completion_bonus" type="number" value={pointsCompletionBonus}
        oninput={(e) => (pointsCompletionBonus = (e.target as HTMLInputElement).value)}
        class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3 text-base text-brand-text focus:border-brand-blue focus:outline-none" />
    </div>
  </div>
</div>

<!-- Intervals overrides (blank = inherit/preserve) -->
<div class="mt-5 border-t border-brand-border pt-4">
  <h3 class="mb-3 text-base font-semibold text-brand-text">
    Intervals <span class="text-xs font-normal text-brand-muted">(blank = {mode === "add" ? "inherit project default" : "preserve current value"})</span>
  </h3>
  <div class="grid grid-cols-2 gap-3">
    <div>
      <label for="{pf}-intervals-deadline" class="mb-1 block text-xs font-semibold text-brand-text">Deadline (secs)</label>
      <input id="{pf}-intervals-deadline" name="intervals_deadline_secs" type="number" value={intervalsDeadline}
        oninput={(e) => (intervalsDeadline = (e.target as HTMLInputElement).value)}
        class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3 text-base text-brand-text focus:border-brand-blue focus:outline-none" />
    </div>
    <div>
      <label for="{pf}-intervals-min" class="mb-1 block text-xs font-semibold text-brand-text">Min interval (secs)</label>
      <input id="{pf}-intervals-min" name="intervals_min_interval_secs" type="number" value={intervalsMin}
        oninput={(e) => (intervalsMin = (e.target as HTMLInputElement).value)}
        class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3 text-base text-brand-text focus:border-brand-blue focus:outline-none" />
    </div>
    <div>
      <label for="{pf}-intervals-increment" class="mb-1 block text-xs font-semibold text-brand-text">Increment (secs)</label>
      <input id="{pf}-intervals-increment" name="intervals_interval_increment_secs" type="number" value={intervalsIncrement}
        oninput={(e) => (intervalsIncrement = (e.target as HTMLInputElement).value)}
        class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3 text-base text-brand-text focus:border-brand-blue focus:outline-none" />
    </div>
    <div>
      <label for="{pf}-intervals-max" class="mb-1 block text-xs font-semibold text-brand-text">Max interval (secs)</label>
      <input id="{pf}-intervals-max" name="intervals_max_interval_secs" type="number" value={intervalsMax}
        oninput={(e) => (intervalsMax = (e.target as HTMLInputElement).value)}
        class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3 text-base text-brand-text focus:border-brand-blue focus:outline-none" />
    </div>
  </div>
</div>
