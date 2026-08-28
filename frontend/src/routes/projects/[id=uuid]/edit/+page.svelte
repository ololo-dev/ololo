<script lang="ts">
  import { enhance } from '$app/forms';
  import { browser } from '$app/environment';
  import type { PageData, ActionData } from './$types';
  import { Plus } from 'lucide-svelte';
  import MarkdownField from '$lib/components/MarkdownField.svelte';
  import TagInput from '$lib/components/TagInput.svelte';
  import CoverImageUpload from '$lib/components/CoverImageUpload.svelte';
  import AccordionSection from '$lib/components/AccordionSection.svelte';
  import TaskRow from '$lib/components/TaskRow.svelte';
  import TaskDrawer from '$lib/components/TaskDrawer.svelte';
  import GeneratedTasksPreview from '$lib/components/GeneratedTasksPreview.svelte';
  import ProjectPointsFields from '$lib/components/ProjectPointsFields.svelte';
  import ProjectIntervalsFields from '$lib/components/ProjectIntervalsFields.svelte';
  import ProjectSessionDurationField from '$lib/components/ProjectSessionDurationField.svelte';
  import ProjectMemorySchemaFields from '$lib/components/ProjectMemorySchemaFields.svelte';
  import EditAiForms from './EditAiForms.svelte';
  import { untrack } from 'svelte';
  import type { TaskDraft } from '$lib/api';

  let { data, form }: { data: PageData; form: ActionData } = $props();

  // Section open states
  let detailsOpen = $state(true);
  let publishOpen = $state(true);
  let tasksOpen = $state(true);

  // Drawer open state: null = closed, "new" = add mode, UUID = edit mode
  let selectedTaskId = $state<string | null>(null);

  // ── AI: description state ──────────────────────────────────────────────────
  let draftDescription = $state(untrack(() => data.project.description ?? ''));
  $effect(() => { draftDescription = data.project.description ?? ''; });
  let beautifyLoading = $state(false);
  let beautifyError = $state<string | null>(null);
  let generateLoading = $state(false);
  let generateError = $state<string | null>(null);

  // Project metadata fields
  let editCategory = $state(untrack(() => data.project.category ?? ''));
  let editTags = $state<string[]>(untrack(() => data.project.tags ?? []));
  let editCoverImageUrl = $state<string | null>(untrack(() => data.project.cover_image_url ?? null));
  let originalCoverImageUrl = untrack(() => data.project.cover_image_url ?? null);
  let coverImageCleared = $state(false);

  // Project points defaults (admin-editable). Empty = preserve on PATCH.
  let editPointsValue = $state(untrack(() => String(data.project.points?.value ?? '')));
  let editPointsFail = $state(untrack(() => String(data.project.points?.fail ?? '')));
  let editPointsNoResponse = $state(untrack(() => String(data.project.points?.no_response ?? '')));
  let editPointsCompletionBonus = $state(untrack(() => String(data.project.points?.completion_bonus ?? '')));
  let pointsOpen = $state(false);

  // Project intervals defaults (admin-editable). Empty = preserve on PATCH.
  let editIntervalsDeadline = $state(untrack(() => String(data.project.intervals?.deadline_secs ?? '')));
  let editIntervalsMin = $state(untrack(() => String(data.project.intervals?.min_interval_secs ?? '')));
  let editIntervalsIncrement = $state(untrack(() => String(data.project.intervals?.interval_increment_secs ?? '')));
  let editIntervalsMax = $state(untrack(() => String(data.project.intervals?.max_interval_secs ?? '')));
  let intervalsOpen = $state(false);

  // Project session-duration default (admin-editable). Empty = preserve on PATCH.
  let editSessionDuration = $state(untrack(() => String(data.project.session_duration_secs ?? '')));
  let editIdleTimeout = $state(untrack(() => String(data.project.idle_timeout_secs ?? '')));

  // Session-memory schema, edited as key/default rows (project owners too).
  function memoryRows(schema: Record<string, string | number | boolean> | null | undefined) {
    return Object.entries(schema ?? {}).map(([key, value]) => ({ key, value: String(value) }));
  }
  let editMemoryRows = $state(untrack(() => memoryRows(data.project.memory_schema)));
  let memoryOpen = $state(false);

  $effect(() => {
    editCategory = data.project.category ?? '';
    editTags = data.project.tags ?? [];
    editCoverImageUrl = data.project.cover_image_url ?? null;
    originalCoverImageUrl = data.project.cover_image_url ?? null;
    coverImageCleared = false;
    editPointsValue = String(data.project.points?.value ?? '');
    editPointsFail = String(data.project.points?.fail ?? '');
    editPointsNoResponse = String(data.project.points?.no_response ?? '');
    editPointsCompletionBonus = String(data.project.points?.completion_bonus ?? '');
    editIntervalsDeadline = String(data.project.intervals?.deadline_secs ?? '');
    editIntervalsMin = String(data.project.intervals?.min_interval_secs ?? '');
    editIntervalsIncrement = String(data.project.intervals?.interval_increment_secs ?? '');
    editIntervalsMax = String(data.project.intervals?.max_interval_secs ?? '');
    editSessionDuration = String(data.project.session_duration_secs ?? '');
    editIdleTimeout = String(data.project.idle_timeout_secs ?? '');
    editMemoryRows = memoryRows(data.project.memory_schema);
  });

  function handleCoverImageChange(url: string | null) {
    editCoverImageUrl = url;
    if (url === null && originalCoverImageUrl !== null) {
      coverImageCleared = true;
    } else {
      coverImageCleared = false;
    }
  }

  // ── AI: generated tasks preview ────────────────────────────────────────────
  let draftTasks = $state<TaskDraft[] | null>(null);
  let importResult = $state<{ imported: number; failed: number; total: number } | null>(null);

  // Task ordering
  let taskOrder = $state<string[]>([]);
  $effect(() => { taskOrder = data.tasks.map((t) => t.id); });

  function orderedTasks() {
    return taskOrder
      .map((id) => data.tasks.find((t) => t.id === id))
      .filter(Boolean) as typeof data.tasks;
  }

  function moveTask(taskId: string, direction: 'up' | 'down') {
    const ids = [...taskOrder];
    const idx = ids.indexOf(taskId);
    if (direction === 'up' && idx > 0) {
      [ids[idx - 1], ids[idx]] = [ids[idx], ids[idx - 1]];
    } else if (direction === 'down' && idx < ids.length - 1) {
      [ids[idx], ids[idx + 1]] = [ids[idx + 1], ids[idx]];
    }
    taskOrder = ids;
    const rf = document.getElementById('reorder-form') as HTMLFormElement | null;
    if (rf) {
      (rf.querySelector('input[name="task_ids"]') as HTMLInputElement).value = ids.join(',');
      rf.requestSubmit();
    }
  }

  // Drag-and-drop ordering
  let draggedId = $state<string | null>(null);
  let dragOverId = $state<string | null>(null);

  function dropTask(targetId: string) {
    if (!draggedId || draggedId === targetId) return;
    const ids = [...taskOrder];
    const fromIdx = ids.indexOf(draggedId);
    const toIdx = ids.indexOf(targetId);
    if (fromIdx === -1 || toIdx === -1) return;
    ids.splice(fromIdx, 1);
    ids.splice(toIdx, 0, draggedId);
    taskOrder = ids;
    const rf = document.getElementById('reorder-form') as HTMLFormElement | null;
    if (rf) {
      (rf.querySelector('input[name="task_ids"]') as HTMLInputElement).value = ids.join(',');
      rf.requestSubmit();
    }
  }
</script>

<svelte:head>
  <title>Edit project — ololo.dev</title>
</svelte:head>

<!-- Full-bleed wrapper -->
<div class="-mx-6 -mt-8 min-h-screen bg-brand-light-blue">
  <div class="mx-auto w-full max-w-[1206px] px-[18px] py-10 md:py-[88px]">

    <!-- Page title -->
    <h1 class="font-heading text-[34px] font-bold leading-[1.18] text-brand-text">
      Edit {data.project.name}
    </h1>

    <!-- ── Project form: wraps Details + Publish ──────────────────────────── -->
    <form id="project-form" method="POST" action="?/editProject" use:enhance>

      <!-- Action bar -->
      <div class="mt-8 flex items-center justify-between border-b-2 border-brand-border pb-3">
        <span></span>
        <div class="flex items-center gap-3">
          <a
            href="/projects/{data.project.id}"
            class="text-sm font-semibold text-brand-blue transition-opacity hover:opacity-70"
          >
            Cancel
          </a>
          <button
            type="submit"
            data-testid="save-project"
            class="rounded-btn bg-brand-blue px-6 py-2 text-sm font-semibold text-white
                   transition-opacity hover:opacity-80"
          >
            Save project
          </button>
        </div>
      </div>

      <!-- ── Details accordion ──────────────────────────────────────────── -->
      <AccordionSection bind:open={detailsOpen} title="Details">
        <div class="rounded-[8px] bg-white">
          <div class="px-[100px] py-6 max-md:px-6">

            <!-- Name -->
            <div class="mb-4">
              <label for="project-name" class="mb-1 block text-xs font-semibold text-brand-text">
                Project name
              </label>
              <input
                id="project-name"
                name="name"
                type="text"
                required
                maxlength="200"
                value={data.project.name}
                placeholder="e.g. My coding challenge"
                class="h-[48px] w-full rounded-[8px] border-2 border-brand-border bg-white px-4
                       text-base text-brand-text placeholder:text-brand-muted
                       focus:border-brand-blue focus:outline-none"
                data-testid="project-name"
              />
            </div>

            <!-- Description -->
            <div>
              <div class="mb-1 flex items-center justify-between">
                <div class="text-xs font-semibold text-brand-text">Description</div>
                <div class="flex items-center gap-2">
                  <button
                    type="button"
                    disabled={beautifyLoading}
                    onclick={() => (document.getElementById('beautify-form') as HTMLFormElement | null)?.requestSubmit()}
                    class="rounded border border-brand-border px-2 py-0.5 text-xs text-brand-text
                           transition-colors hover:bg-brand-light-blue disabled:cursor-not-allowed disabled:opacity-50"
                    data-testid="beautify-btn"
                  >
                    {beautifyLoading ? 'Beautifying…' : 'Beautify'}
                  </button>
                  <button
                    type="button"
                    disabled={generateLoading}
                    onclick={() => (document.getElementById('generate-tasks-form') as HTMLFormElement | null)?.requestSubmit()}
                    class="rounded border border-brand-border px-2 py-0.5 text-xs text-brand-text
                           transition-colors hover:bg-brand-light-blue disabled:cursor-not-allowed disabled:opacity-50"
                    data-testid="generate-tasks-btn"
                  >
                    {generateLoading ? 'Generating…' : 'Generate Tasks'}
                  </button>
                </div>
              </div>
              {#if beautifyError}
                <p class="mb-1 text-xs text-brand-red">{beautifyError}</p>
              {/if}
              {#if generateError}
                <p class="mb-1 text-xs text-brand-red">{generateError}</p>
              {/if}
              <div oninput={(e) => { draftDescription = (e.target as HTMLTextAreaElement).value; }}>
                <MarkdownField
                  name="description"
                  value={draftDescription}
                  placeholder="Optional project description"
                  data-testid="project-description"
                />
              </div>
            </div>

            <!-- Category -->
            <div class="mb-4 mt-4">
              <label for="edit-category" class="mb-1 block text-xs font-semibold text-brand-text">
                Category <span class="font-normal text-brand-muted">(optional)</span>
              </label>
              <select
                id="edit-category"
                name="category"
                value={editCategory}
                onchange={(e) => (editCategory = (e.target as HTMLSelectElement).value)}
                class="h-[48px] w-full rounded-[8px] border-2 border-brand-border bg-white px-4
                       text-base text-brand-text focus:border-brand-blue focus:outline-none"
              >
                <option value="">No category</option>
                {#each data.categories as cat}
                  <option value={cat}>{cat}</option>
                {/each}
              </select>
            </div>

            <!-- Tags -->
            <div class="mb-4">
              <div class="mb-1 text-xs font-semibold text-brand-text">
                Tags <span class="font-normal text-brand-muted">(optional, max 10)</span>
              </div>
              <TagInput tags={editTags} onchange={(t) => (editTags = t)} />
              <input type="hidden" name="tags_json" value={JSON.stringify(editTags)} />
            </div>

            <!-- Cover image -->
            <div class="mb-4">
              <div class="mb-1 text-xs font-semibold text-brand-text">
                Cover image <span class="font-normal text-brand-muted">(optional)</span>
              </div>
              <CoverImageUpload value={editCoverImageUrl} onchange={handleCoverImageChange} />
              <input type="hidden" name="cover_image_url" value={editCoverImageUrl ?? ''} />
              <input type="hidden" name="clear_cover_image" value={String(coverImageCleared)} />
            </div>

            <!-- Session-memory schema (owner-editable, collapsible) -->
            <ProjectMemorySchemaFields
              open={memoryOpen}
              onOpenChange={(v) => (memoryOpen = v)}
              rows={editMemoryRows}
              onchange={(rows) => (editMemoryRows = rows)}
              idPrefix="edit"
            />

            <!-- Points defaults (admin only, collapsible) -->
            {#if data.isAdmin}
              <ProjectPointsFields
                bind:open={pointsOpen}
                bind:value={editPointsValue}
                bind:fail={editPointsFail}
                bind:noResponse={editPointsNoResponse}
                bind:completionBonus={editPointsCompletionBonus}
                baseline="task inheritance baseline"
                idPrefix="edit"
              />
            {/if}

            <!-- Intervals defaults (admin only, collapsible) -->
            {#if data.isAdmin}
              <ProjectIntervalsFields
                bind:open={intervalsOpen}
                bind:deadline={editIntervalsDeadline}
                bind:min={editIntervalsMin}
                bind:increment={editIntervalsIncrement}
                bind:max={editIntervalsMax}
                baseline="task inheritance baseline"
                idPrefix="edit"
              />
            {/if}

            <!-- Session duration default (admin only) -->
            {#if data.isAdmin}
              <ProjectSessionDurationField
                value={editSessionDuration}
                onchange={(v) => (editSessionDuration = v)}
                idleValue={editIdleTimeout}
                onIdleChange={(v) => (editIdleTimeout = v)}
                idPrefix="edit"
              />
            {/if}

            {#if form?.action === 'editProject' && form?.error}
              <p class="mt-4 text-sm text-brand-red">Error: {form.error}</p>
            {/if}

          </div>
        </div>
      </AccordionSection>

      <!-- ── Publish accordion — admin only ─────────────────────────────── -->
      {#if data.isAdmin}
      <AccordionSection bind:open={publishOpen} title="Publish">
        <div class="rounded-[8px] bg-white">
          <div class="px-[100px] py-6 max-md:px-6">

            <!-- Public checkbox -->
            <div class="mb-4 flex items-start gap-3">
              <input
                id="project-public"
                name="public"
                type="checkbox"
                value="true"
                checked={data.project.public ?? false}
                class="mt-0.5 rounded border-brand-border"
                data-testid="project-public"
              />
              <div>
                <label for="project-public" class="text-sm font-semibold text-brand-text">
                  Public project
                </label>
                <p class="mt-0.5 text-xs text-brand-muted">Share your project publicly</p>
              </div>
            </div>

            <!-- Slug (admin only) -->
            <div>
              <label for="project-slug" class="mb-1 block text-xs font-semibold text-brand-text">
                URL slug
              </label>
              <input
                id="project-slug"
                name="slug"
                type="text"
                maxlength="64"
                value={data.project.slug ?? ''}
                placeholder="my-coding-challenge"
                class="h-[48px] w-full rounded-[8px] border-2 border-brand-border bg-white px-4
                       font-mono text-sm text-brand-text placeholder:text-brand-muted
                       focus:border-brand-blue focus:outline-none"
                data-testid="project-slug"
              />
              <p class="mt-1.5 text-xs text-brand-muted">
                URL-safe identifier (lowercase letters, digits, hyphens). Leave blank to remove.
              </p>
            </div>

          </div>

          <!-- Card footer: save button flush right -->
          <div class="flex justify-end border-t border-brand-border px-[100px] py-4 max-md:px-6">
            <button
              type="submit"
              class="rounded-btn bg-brand-blue px-6 py-2 text-sm font-semibold text-white
                     transition-opacity hover:opacity-80"
            >
              Save project
            </button>
          </div>
        </div>
      </AccordionSection>
      {/if}

    </form>

    <!-- ── Tasks accordion (separate from project form) ───────────────────── -->
    <AccordionSection bind:open={tasksOpen} title="Tasks" count={data.tasks.length}>
      <div class="overflow-hidden rounded-[8px] bg-white">

        {#if data.tasks.length > 0}
          <!-- Column headers -->
          <div class="grid grid-cols-[2rem_3rem_1fr_10rem_auto] items-center gap-4 border-b border-brand-border px-5 py-2 text-xs text-brand-muted">
            <div></div>
            <div>#</div>
            <div>Title</div>
            <div>Tags</div>
            <div>Actions</div>
          </div>

          <!-- Hidden reorder form -->
          <form id="reorder-form" method="POST" action="?/reorderTasks" use:enhance class="hidden">
            <input type="hidden" name="task_ids" value={taskOrder.join(',')} />
          </form>

          <!-- Task rows -->
          <ol>
            {#each orderedTasks() as task, i (task.id)}
              <TaskRow
                {task}
                index={i}
                total={orderedTasks().length}
                draggedId={draggedId}
                dragOverId={dragOverId}
                onMove={moveTask}
                onEdit={(id) => { selectedTaskId = id; draftTasks = null; importResult = null; }}
                onDragStart={(id) => { draggedId = id; }}
                onDragEnd={() => { draggedId = null; dragOverId = null; }}
                onDragOver={(id) => { dragOverId = id; }}
                onDragLeave={(id) => { if (dragOverId === id) dragOverId = null; }}
                onDrop={(id) => { dropTask(id); }}
              />
            {/each}
          </ol>
        {:else}
          <div class="py-10 text-center text-sm text-brand-muted">No tasks yet.</div>
        {/if}

        <!-- Footer: add task -->
        <div class="flex justify-end border-t border-brand-border px-5 py-3">
          <button
            type="button"
            onclick={() => { selectedTaskId = 'new'; draftTasks = null; importResult = null; }}
            class="flex items-center gap-1.5 rounded-btn border border-brand-border px-3 py-1.5
                   text-sm text-brand-text transition-colors hover:bg-brand-light-blue"
            data-testid="add-task-btn"
          >
            <Plus size={14} /> Add task
          </button>
        </div>

      </div>
    </AccordionSection>

  </div>
</div>

<!-- Task add/edit drawer -->
<TaskDrawer {data} {form} bind:selectedTaskId />

<!-- Generated tasks preview drawer -->
<GeneratedTasksPreview
  draftTasks={draftTasks}
  importResult={importResult}
  onImport={() => (document.getElementById('import-tasks-form') as HTMLFormElement | null)?.requestSubmit()}
  onClose={() => { draftTasks = null; importResult = null; }}
/>

<!-- ═══════════════════════════════════════════════════════════════════════ -->
<!-- Hidden AI action forms (project level) — extracted to EditAiForms       -->
<!-- ═══════════════════════════════════════════════════════════════════════ -->

<EditAiForms
  bind:draftDescription
  bind:draftTasks
  bind:importResult
  bind:beautifyLoading
  bind:beautifyError
  bind:generateLoading
  bind:generateError
  onGenerated={() => { selectedTaskId = null; }}
/>
