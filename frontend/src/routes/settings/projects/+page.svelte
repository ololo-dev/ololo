<script lang="ts">
  import { invalidateAll } from '$app/navigation';
  import { notify } from '$lib/notifications.svelte';
  import {
    importProject,
    exportProject,
    deleteProject,
    patchProject,
    reseedProject,
    type Project,
  } from '$lib/api';

  let { data } = $props();

  let syncingId = $state<string | null>(null);

  /** Re-read the project's on-disk seed definition (matched by slug). */
  async function onSyncProject(project: Project) {
    if (syncingId) return;
    syncingId = project.id;
    try {
      const r = await reseedProject(project.id, { fetch });
      notify.success(
        `"${r.name}" synced — tasks: ${r.tasks_updated} updated, ${r.tasks_inserted} added, ${r.tasks_deleted} removed`,
        'Sync',
      );
      await invalidateAll();
    } catch (err) {
      notify.error(err instanceof Error ? err.message : String(err), 'Sync failed');
    } finally {
      syncingId = null;
    }
  }

  async function onImportProject(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    try {
      const result = await importProject(file, { fetch });
      notify.success(`Imported "${result.name}"`, 'Project');
      await invalidateAll();
    } catch (err) {
      notify.error(err instanceof Error ? err.message : String(err), 'Import failed');
    }
    input.value = '';
  }

  async function onExportProject(project: Project) {
    try {
      const exported = await exportProject(project.id, { fetch });
      const blob = new Blob([JSON.stringify(exported, null, 2)], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = (exported.project.slug ?? project.name) + '.json';
      a.click();
      URL.revokeObjectURL(url);
      notify.success(`Downloaded "${project.name}"`, 'Export');
    } catch (err) {
      notify.error(err instanceof Error ? err.message : String(err), 'Export failed');
    }
  }

  async function onArchiveProject(project: Project) {
    try {
      await patchProject(project.id, { archived: true }, { fetch });
      notify.success(`Archived "${project.name}"`, 'Project');
      await invalidateAll();
    } catch (err) {
      notify.error(err instanceof Error ? err.message : String(err), 'Archive failed');
    }
  }

  async function onUnarchiveProject(project: Project) {
    try {
      await patchProject(project.id, { archived: false }, { fetch });
      notify.success(`Unarchived "${project.name}"`, 'Project');
      await invalidateAll();
    } catch (err) {
      notify.error(err instanceof Error ? err.message : String(err), 'Unarchive failed');
    }
  }

  async function onDeleteProject(project: Project) {
    if (!confirm(`Delete "${project.name}"? This cannot be undone.`)) return;
    try {
      await deleteProject(project.id, { fetch });
      notify.success(`Deleted "${project.name}"`, 'Project');
      await invalidateAll();
    } catch (err) {
      notify.error(err instanceof Error ? err.message : String(err), 'Delete failed');
    }
  }
</script>

<div class="mt-8">
  <div class="mb-4 flex flex-wrap items-center justify-between gap-3">
    <div>
      <h2 class="font-heading text-[20px] font-semibold text-brand-text">
        Projects
      </h2>
      <p class="mt-0.5 text-sm text-brand-muted">
        {data.projects.length} {data.projects.length === 1 ? 'project' : 'projects'} on this instance.
      </p>
    </div>
    <div oninput={onImportProject}>
      <label
        for="import-project-input"
        class="cursor-pointer rounded-btn bg-brand-blue px-5 py-2 text-sm font-semibold text-white
               transition-opacity hover:opacity-80"
      >
        Import Project
      </label>
      <input
        id="import-project-input"
        type="file"
        accept="application/json"
        class="sr-only"
      />
    </div>
  </div>

  {#snippet visibilityBadge(project: Project)}
    {#if project.public}
      <span class="rounded-full bg-green-100 px-2 py-0.5 text-xs font-semibold text-green-700">Public</span>
    {:else}
      <span class="rounded-full bg-gray-100 px-2 py-0.5 text-xs font-semibold text-gray-600">Private</span>
    {/if}
  {/snippet}

  {#snippet statusBadge(project: Project)}
    {#if project.archived_at}
      <span class="rounded-full bg-yellow-100 px-2 py-0.5 text-xs font-semibold text-yellow-700">Archived</span>
    {:else}
      <span class="rounded-full bg-blue-100 px-2 py-0.5 text-xs font-semibold text-blue-700">Active</span>
    {/if}
  {/snippet}

  {#snippet actions(project: Project)}
    <a
      href="/projects/{project.id}/edit"
      class="rounded px-3 py-1 text-xs font-semibold text-brand-blue
             transition-colors hover:bg-brand-blue/10"
    >
      Edit
    </a>
    <button
      type="button"
      onclick={() => onExportProject(project)}
      class="rounded px-3 py-1 text-xs font-semibold text-brand-blue
             transition-colors hover:bg-brand-blue/10"
    >
      Export
    </button>
    {#if project.slug}
      <button
        type="button"
        data-testid="sync-project-btn"
        onclick={() => onSyncProject(project)}
        disabled={syncingId !== null}
        title="Re-read the on-disk seed definition (matched by slug) and update this project in place"
        class="rounded px-3 py-1 text-xs font-semibold text-brand-blue
               transition-colors hover:bg-brand-blue/10 disabled:opacity-50"
      >
        {syncingId === project.id ? 'Syncing…' : 'Sync'}
      </button>
    {/if}
    {#if project.archived_at}
      <button
        type="button"
        onclick={() => onUnarchiveProject(project)}
        class="rounded px-3 py-1 text-xs font-semibold text-brand-blue
               transition-colors hover:bg-brand-blue/10"
      >
        Unarchive
      </button>
    {:else}
      <button
        type="button"
        onclick={() => onArchiveProject(project)}
        class="rounded px-3 py-1 text-xs font-semibold text-brand-blue
               transition-colors hover:bg-brand-blue/10"
      >
        Archive
      </button>
    {/if}
    <button
      type="button"
      onclick={() => onDeleteProject(project)}
      class="rounded px-3 py-1 text-xs font-semibold text-red-500
             transition-colors hover:bg-red-50"
    >
      Delete
    </button>
  {/snippet}

  <div class="rounded-[8px] bg-white shadow-sm">
    {#if data.projects.length === 0}
      <div class="flex flex-col items-center justify-center py-16 text-brand-muted">
        <p class="text-sm">No projects yet.</p>
      </div>
    {:else}
      <!-- Cards below `xl`, same pattern as the sessions page: six columns
           plus five action buttons do not fit a phone, and a sideways-scrolling
           admin table is worse than a stacked card. -->
      <ul class="divide-y divide-brand-border/60 xl:hidden">
        {#each data.projects as project (project.id)}
          <li class="flex flex-col gap-2.5 p-4">
            <div class="flex items-start justify-between gap-3">
              <span class="flex min-w-0 flex-col leading-tight">
                <a
                  href="/projects/{project.slug ?? project.id}"
                  class="truncate font-semibold text-brand-text hover:underline"
                >{project.name}</a>
                {#if project.slug}
                  <span class="truncate text-xs text-brand-muted">/{project.slug}</span>
                {/if}
              </span>
              <span class="flex shrink-0 gap-1.5">
                {@render visibilityBadge(project)}
                {@render statusBadge(project)}
              </span>
            </div>

            <dl class="grid grid-cols-[4.5rem_minmax(0,1fr)] items-center gap-x-3 gap-y-1.5 text-xs text-brand-muted">
              <dt class="font-semibold uppercase tracking-wider text-brand-muted/70">Category</dt>
              <dd class="min-w-0 truncate">{project.category ?? '—'}</dd>
              <dt class="font-semibold uppercase tracking-wider text-brand-muted/70">Tasks</dt>
              <dd>{project.task_count}</dd>
            </dl>

            <div class="flex flex-wrap justify-end gap-1">
              {@render actions(project)}
            </div>
          </li>
        {/each}
      </ul>

      <div class="hidden xl:block">
        <table class="w-full table-fixed">
          <thead>
            <tr class="border-b border-brand-border bg-brand-light-blue/40">
              <th class="px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted first:pl-6">Name</th>
              <th class="w-[110px] px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Category</th>
              <th class="w-[64px] px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Tasks</th>
              <th class="w-[88px] px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Visibility</th>
              <th class="w-[88px] px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Status</th>
              <th class="w-[300px] py-3 pl-4 pr-6 text-right text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Actions</th>
            </tr>
          </thead>
          <tbody>
            {#each data.projects as project (project.id)}
              <tr class="border-b border-brand-border/60 last:border-0 hover:bg-brand-light-blue/30 transition-colors">
                <td class="px-4 py-4 first:pl-6">
                  <span class="flex min-w-0 flex-col leading-tight">
                    <a
                      href="/projects/{project.slug ?? project.id}"
                      class="truncate font-semibold text-brand-text hover:underline"
                      title={project.name}
                    >{project.name}</a>
                    {#if project.slug}
                      <span class="truncate text-xs text-brand-muted">/{project.slug}</span>
                    {/if}
                  </span>
                </td>
                <td class="truncate px-4 py-4 text-sm text-brand-muted" title={project.category ?? undefined}>
                  {project.category ?? '—'}
                </td>
                <td class="px-4 py-4 text-sm tabular-nums text-brand-muted">
                  {project.task_count}
                </td>
                <td class="px-4 py-4 text-sm">
                  {@render visibilityBadge(project)}
                </td>
                <td class="px-4 py-4 text-sm">
                  {@render statusBadge(project)}
                </td>
                <td class="py-4 pl-4 pr-6 text-right">
                  <div class="flex items-center justify-end gap-1">
                    {@render actions(project)}
                  </div>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </div>
</div>