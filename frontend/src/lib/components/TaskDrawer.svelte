<script lang="ts">
  import { browser } from '$app/environment';
  import { invalidateAll } from '$app/navigation';
  import { enhance } from '$app/forms';
  import { untrack } from 'svelte';
  import { Drawer } from 'vaul-svelte';
  import {
    type TaskJudge,
    type Judge,
    type Project,
    type Task,
  } from '$lib/api';
  import JudgesPanel from '$lib/components/JudgesPanel.svelte';
  import TaskFormFields from '$lib/components/TaskFormFields.svelte';

  interface TaskDrawerData {
    project: Project;
    tasks: Task[];
    judges?: Judge[];
    taskJudgesMap: Record<string, TaskJudge[]>;
    isAdmin: boolean;
  }

  interface EditProjectActionData {
    action?: string | null;
    error?: string | null;
  }

  let {
    data,
    form,
    selectedTaskId = $bindable(),
  }: {
    data: TaskDrawerData;
    form: EditProjectActionData | null;
    selectedTaskId: string | null;
  } = $props();

  // Drawer field state
  let drawerDescription = $state('');
  let drawerCommandTemplate = $state('');
  let drawerAnswerTemplate = $state('');
  let drawerFixtures = $state('[]');
  let drawerTags = $state<string[]>([]);
  let drawerTagInput = $state('');
  let addTaskTitle = $state('');

  // Task drawer points (empty = inherit/preserve)
  let drawerPointsValue = $state('');
  let drawerPointsFail = $state('');
  let drawerPointsNoResponse = $state('');
  let drawerPointsCompletionBonus = $state('');

  // Task drawer intervals (empty = inherit/preserve)
  let drawerIntervalsDeadline = $state('');
  let drawerIntervalsMin = $state('');
  let drawerIntervalsIncrement = $state('');
  let drawerIntervalsMax = $state('');

  // Add/edit task AI loading
  let addBeautifyLoading = $state(false);
  let addBeautifyError = $state<string | null>(null);
  let addGenerateLoading = $state(false);
  let addGenerateError = $state<string | null>(null);
  let editBeautifyLoading = $state(false);
  let editBeautifyError = $state<string | null>(null);
  let editGenerateLoading = $state(false);
  let editGenerateError = $state<string | null>(null);

  $effect(() => {
    if (selectedTaskId === 'new') {
      drawerDescription = '';
      drawerCommandTemplate = '';
      drawerAnswerTemplate = '';
      drawerFixtures = '[]';
      drawerTags = [];
      drawerTagInput = '';
      addTaskTitle = '';
      drawerPointsValue = '';
      drawerPointsFail = '';
      drawerPointsNoResponse = '';
      drawerPointsCompletionBonus = '';
      drawerIntervalsDeadline = '';
      drawerIntervalsMin = '';
      drawerIntervalsIncrement = '';
      drawerIntervalsMax = '';
    } else if (selectedTaskId) {
      const task = data.tasks.find((t) => t.id === selectedTaskId);
      drawerDescription = task?.description ?? '';
      drawerCommandTemplate = task?.test_template.command_template ?? '';
      drawerAnswerTemplate = task?.test_template.answer_template ?? '';
      drawerFixtures = JSON.stringify(task?.test_template.fixtures ?? []);
      drawerTags = untrack(() => task?.tags ?? []);
      drawerTagInput = '';
      addTaskTitle = '';
      drawerPointsValue = String(task?.points?.value ?? '');
      drawerPointsFail = String(task?.points?.fail ?? '');
      drawerPointsNoResponse = String(task?.points?.no_response ?? '');
      drawerPointsCompletionBonus = String(task?.points?.completion_bonus ?? '');
      drawerIntervalsDeadline = String(task?.intervals?.deadline_secs ?? '');
      drawerIntervalsMin = String(task?.intervals?.min_interval_secs ?? '');
      drawerIntervalsIncrement = String(task?.intervals?.interval_increment_secs ?? '');
      drawerIntervalsMax = String(task?.intervals?.max_interval_secs ?? '');
    }
  });

  const selectedTask = $derived(
    selectedTaskId && selectedTaskId !== 'new'
      ? (data.tasks.find((t) => t.id === selectedTaskId) ?? null)
      : null,
  );
</script>

{#if browser}
<Drawer.Root
  open={selectedTaskId !== null}
  onOpenChange={(v) => { if (!v) selectedTaskId = null; }}
  direction="right"
>
  <Drawer.Portal>
    <Drawer.Overlay class="fixed inset-0 z-50 bg-black/80" />
    <Drawer.Content
      class="fixed right-0 top-0 bottom-0 z-50 flex h-full w-[70vw] flex-col bg-white shadow-2xl"
    >
      <!-- Header -->
      <div class="border-b border-brand-border px-6 py-4">
        <h2 class="font-heading text-xl font-bold text-brand-text">
          {selectedTaskId === 'new' ? 'Add task' : 'Edit task'}
        </h2>
        {#if selectedTask}
          <p class="mt-0.5 text-sm text-brand-muted">
            {data.project.name} <span class="text-brand-muted">›</span> #{selectedTask.ordinal} {selectedTask.title}
          </p>
        {:else}
          <p class="mt-0.5 text-sm text-brand-muted">{data.project.name}</p>
        {/if}
      </div>

      <!-- Form content -->
      <div class="flex-1 overflow-y-auto px-6 py-5">
        {#if selectedTaskId === 'new'}
          <!-- ── Add Task form ── -->
          <form
            method="POST"
            action="?/addTask"
            use:enhance={() => async ({ result, update }) => {
              if (result.type === 'success') { selectedTaskId = null; await invalidateAll(); }
              else { await update(); }
            }}
            class="space-y-5"
          >
            <TaskFormFields
              mode="add"
              {selectedTask}
              bind:addTitle={addTaskTitle}
              bind:description={drawerDescription}
              bind:commandTemplate={drawerCommandTemplate}
              bind:tags={drawerTags}
              bind:tagInput={drawerTagInput}
              bind:pointsValue={drawerPointsValue}
              bind:pointsFail={drawerPointsFail}
              bind:pointsNoResponse={drawerPointsNoResponse}
              bind:pointsCompletionBonus={drawerPointsCompletionBonus}
              bind:intervalsDeadline={drawerIntervalsDeadline}
              bind:intervalsMin={drawerIntervalsMin}
              bind:intervalsIncrement={drawerIntervalsIncrement}
              bind:intervalsMax={drawerIntervalsMax}
              bind:beautifyLoading={addBeautifyLoading}
              bind:beautifyError={addBeautifyError}
              bind:generateLoading={addGenerateLoading}
              bind:generateError={addGenerateError}
              beautifyFormId="add-task-beautify-form"
              generateFormId="add-task-generate-form"
              answerTemplate={drawerAnswerTemplate}
              fixtures={drawerFixtures}
            />

            {#if form?.action === 'addTask' && form?.error}
              <p class="text-sm text-brand-red">Error: {form.error}</p>
            {/if}

            <div class="flex justify-end gap-2 border-t border-brand-border pt-4">
              <button
                type="button"
                onclick={() => { selectedTaskId = null; }}
                class="rounded-btn border border-brand-border px-4 py-2 text-sm text-brand-text hover:bg-brand-light-blue"
              >Cancel</button>
              <button
                type="submit"
                class="rounded-btn bg-brand-blue px-4 py-2 text-sm font-semibold text-white hover:opacity-80"
                data-testid="save-task"
              >Save task</button>
            </div>
          </form>

        {:else if selectedTask}
          <!-- ── Edit Task form ── -->
          <form
            method="POST"
            action="?/editTask"
            use:enhance={() => async ({ result, update }) => {
              if (result.type === 'success') { selectedTaskId = null; await invalidateAll(); }
              else { await update(); }
            }}
            class="space-y-5"
          >
            <input type="hidden" name="task_id" value={selectedTask.id} />

            <TaskFormFields
              mode="edit"
              {selectedTask}
              bind:addTitle={addTaskTitle}
              bind:description={drawerDescription}
              bind:commandTemplate={drawerCommandTemplate}
              bind:tags={drawerTags}
              bind:tagInput={drawerTagInput}
              bind:pointsValue={drawerPointsValue}
              bind:pointsFail={drawerPointsFail}
              bind:pointsNoResponse={drawerPointsNoResponse}
              bind:pointsCompletionBonus={drawerPointsCompletionBonus}
              bind:intervalsDeadline={drawerIntervalsDeadline}
              bind:intervalsMin={drawerIntervalsMin}
              bind:intervalsIncrement={drawerIntervalsIncrement}
              bind:intervalsMax={drawerIntervalsMax}
              bind:beautifyLoading={editBeautifyLoading}
              bind:beautifyError={editBeautifyError}
              bind:generateLoading={editGenerateLoading}
              bind:generateError={editGenerateError}
              beautifyFormId="edit-task-beautify-form"
              generateFormId="edit-task-generate-form"
              answerTemplate={drawerAnswerTemplate}
              fixtures={drawerFixtures}
            />

            <JudgesPanel
              project={data.project}
              {selectedTaskId}
              judges={data.judges}
              taskJudgesMap={data.taskJudgesMap}
            />

            {#if form?.action === 'editTask' && form?.error}
              <p class="text-sm text-brand-red">Error: {form.error}</p>
            {/if}

            <div class="flex justify-end gap-2 border-t border-brand-border pt-4">
              <button
                type="button"
                onclick={() => { selectedTaskId = null; }}
                class="rounded-btn border border-brand-border px-4 py-2 text-sm text-brand-text hover:bg-brand-light-blue"
              >Cancel</button>
              <button
                type="submit"
                class="rounded-btn bg-brand-blue px-4 py-2 text-sm font-semibold text-white hover:opacity-80"
                data-testid="save-task"
              >Save</button>
            </div>
          </form>
        {/if}
      </div>
    </Drawer.Content>
  </Drawer.Portal>
</Drawer.Root>
{/if}

<!-- ── Task drawer hidden AI action forms ── -->
<!-- Add Task: Beautify Description -->
<form
  id="add-task-beautify-form"
  method="POST"
  action="?/beautifyTaskDescription"
  use:enhance={({ formData, cancel }) => {
    const desc = drawerDescription?.trim();
    if (!desc) { cancel(); return; }
    formData.set('description', desc);
    addBeautifyLoading = true;
    addBeautifyError = null;
    return async ({ result }) => {
      addBeautifyLoading = false;
      if (result.type === 'success' && result.data?.description) {
        drawerDescription = result.data.description as string;
      } else if (result.type === 'failure') {
        addBeautifyError = (result.data?.error as string) ?? 'Beautify failed';
      }
    };
  }}
  class="hidden"
></form>

<!-- Add Task: Generate Tests -->
<form
  id="add-task-generate-form"
  method="POST"
  action="?/generateTaskTests"
  use:enhance={({ formData, cancel }) => {
    const desc = drawerDescription?.trim();
    const title = addTaskTitle.trim();
    if (!desc || !title) { cancel(); return; }
    formData.set('description', desc);
    formData.set('title', title);
    addGenerateLoading = true;
    addGenerateError = null;
    return async ({ result }) => {
      addGenerateLoading = false;
      if (result.type === 'success' && result.data?.command_template) {
        drawerCommandTemplate = result.data.command_template as string;
        drawerAnswerTemplate = (result.data.answer_template as string) ?? '';
        drawerFixtures = JSON.stringify((result.data.fixtures as unknown[]) ?? []);
      } else if (result.type === 'failure') {
        addGenerateError = (result.data?.error as string) ?? 'Generate failed';
      }
    };
  }}
  class="hidden"
></form>

<!-- Edit Task: Beautify Description -->
<form
  id="edit-task-beautify-form"
  method="POST"
  action="?/beautifyTaskDescriptionById"
  use:enhance={({ formData, cancel }) => {
    const desc = drawerDescription?.trim();
    if (!desc || !selectedTaskId) { cancel(); return; }
    formData.set('description', desc);
    formData.set('task_id', selectedTaskId);
    editBeautifyLoading = true;
    editBeautifyError = null;
    return async ({ result }) => {
      editBeautifyLoading = false;
      if (result.type === 'success' && result.data?.description) {
        drawerDescription = result.data.description as string;
      } else if (result.type === 'failure') {
        editBeautifyError = (result.data?.error as string) ?? 'Beautify failed';
      }
    };
  }}
  class="hidden"
></form>

<!-- Edit Task: Generate Tests -->
<form
  id="edit-task-generate-form"
  method="POST"
  action="?/generateTaskTestsById"
  use:enhance={({ formData, cancel }) => {
    const desc = drawerDescription?.trim();
    const taskId = selectedTaskId;
    const title = selectedTask?.title?.trim() ?? '';
    if (!desc || !taskId || !title) { cancel(); return; }
    formData.set('description', desc);
    formData.set('task_id', taskId);
    formData.set('title', title);
    editGenerateLoading = true;
    editGenerateError = null;
    return async ({ result }) => {
      editGenerateLoading = false;
      if (result.type === 'success' && result.data?.command_template) {
        drawerCommandTemplate = result.data.command_template as string;
        drawerAnswerTemplate = (result.data.answer_template as string) ?? '';
        drawerFixtures = JSON.stringify((result.data.fixtures as unknown[]) ?? []);
      } else if (result.type === 'failure') {
        editGenerateError = (result.data?.error as string) ?? 'Generate failed';
      }
    };
  }}
  class="hidden"
></form>
