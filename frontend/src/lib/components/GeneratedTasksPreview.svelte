<script lang="ts">
  import { browser } from '$app/environment';
  import { Drawer } from 'vaul-svelte';
  import type { TaskDraft } from '$lib/api';

  let {
    draftTasks,
    importResult,
    onImport,
    onClose,
  }: {
    draftTasks: TaskDraft[] | null;
    importResult: { imported: number; failed: number; total: number } | null;
    onImport: () => void;
    onClose: () => void;
  } = $props();
</script>

{#if browser}
<Drawer.Root
  open={draftTasks !== null}
  onOpenChange={(v) => { if (!v) onClose(); }}
  direction="right"
>
  <Drawer.Portal>
    <Drawer.Overlay class="fixed inset-0 z-50 bg-black/80" />
    <Drawer.Content
      class="fixed right-0 top-0 bottom-0 z-50 flex h-full w-[80vw] flex-col bg-white shadow-2xl"
    >
      <!-- Header -->
      <div class="border-b border-brand-border px-6 py-4">
        <h2 class="font-heading text-xl font-bold text-brand-text">Generated Tasks Preview</h2>
        <p class="mt-0.5 text-sm text-brand-muted">
          Review tasks before importing — {draftTasks?.length ?? 0} task{draftTasks?.length === 1 ? '' : 's'} generated
        </p>
      </div>

      <!-- Scrollable task list -->
      <div class="flex-1 overflow-y-auto px-6 py-5">
        {#if importResult}
          <div class="mb-4 rounded-[8px] border border-[#dce9fc] bg-[#f4f8fe] px-4 py-3 text-sm text-brand-blue">
            Imported {importResult.imported} of {importResult.total} task{importResult.total === 1 ? '' : 's'}
            {#if importResult.failed > 0}
              <span class="text-brand-red"> — {importResult.failed} failed</span>
            {/if}
          </div>
        {/if}

        <ol class="space-y-2">
          {#each draftTasks ?? [] as task, i}
            <li class="flex gap-3 rounded-[8px] border border-brand-border bg-brand-light-blue px-4 py-3">
              <span class="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-brand-border text-xs font-semibold text-brand-muted">
                {i + 1}
              </span>
              <div>
                <p class="text-sm font-semibold text-brand-text">{task.title}</p>
                {#if task.description}
                  <p class="mt-0.5 text-xs text-brand-muted">{task.description}</p>
                {/if}
                {#if task.tags && task.tags.length > 0}
                  <div class="mt-1.5 flex flex-wrap gap-1">
                    {#each task.tags as tag}
                      <span class="inline-block rounded-[4px] bg-brand-border px-1.5 py-0.5 text-xs text-brand-muted">
                        {tag}
                      </span>
                    {/each}
                  </div>
                {/if}
              </div>
            </li>
          {/each}
        </ol>
      </div>

      <!-- Footer -->
      <div class="flex items-center justify-between border-t border-brand-border px-6 py-4">
        <button
          type="button"
          onclick={onClose}
          class="rounded-btn border border-brand-border px-4 py-2 text-sm text-brand-text hover:bg-brand-light-blue"
        >
          {importResult ? 'Close' : 'Cancel'}
        </button>

        {#if !importResult}
          <button
            type="button"
            onclick={onImport}
            class="rounded-btn bg-brand-blue px-4 py-2 text-sm font-semibold text-white hover:opacity-80"
            data-testid="import-tasks-btn"
          >
            Import {draftTasks?.length ?? 0} task{draftTasks?.length === 1 ? '' : 's'}
          </button>
        {/if}
      </div>
    </Drawer.Content>
  </Drawer.Portal>
</Drawer.Root>
{/if}
