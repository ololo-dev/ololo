<script lang="ts">
  import { enhance } from '$app/forms';
  import { invalidateAll } from '$app/navigation';
  import { untrack } from 'svelte';
  import { notify } from '$lib/notifications.svelte';
  import { reorderCategories, renameCategory, ApiError, type CategoryDto } from '$lib/api';

  let { data } = $props();

  let newCategoryName = $state('');
  let addingCategory = $state(false);
  let addCategoryError = $state<string | undefined>(undefined);
  let deletingCategoryId = $state<number | null>(null);

  let localCategories = $state<CategoryDto[]>(untrack(() => [...data.categories]));
  $effect(() => { localCategories = [...data.categories]; });

  let draggedIdx = $state<number | null>(null);
  let dragOverIdx = $state<number | null>(null);
  let savingOrder = $state(false);

  // ── Inline rename ─────────────────────────────────────────────────────────
  // `projects.category` stores the name, not an FK, so the server rewrites
  // every project filed under the old name in the same transaction; we
  // surface how many moved.
  let editingId = $state<number | null>(null);
  let editingName = $state('');
  let editError = $state<string | undefined>(undefined);
  let savingRename = $state(false);

  function startEdit(cat: CategoryDto) {
    editingId = cat.id;
    editingName = cat.name;
    editError = undefined;
  }

  function cancelEdit() {
    editingId = null;
    editingName = '';
    editError = undefined;
  }

  async function saveEdit(cat: CategoryDto) {
    const name = editingName.trim();
    if (!name) {
      editError = 'Name must be 1–100 characters.';
      return;
    }
    if (name === cat.name) {
      cancelEdit();
      return;
    }
    savingRename = true;
    editError = undefined;
    try {
      const res = await renameCategory(cat.id, name);
      await invalidateAll();
      const moved = res.projects_updated;
      notify.success(
        moved > 0
          ? `Renamed to "${name}" — ${moved} ${moved === 1 ? 'project' : 'projects'} updated.`
          : `Renamed to "${name}".`,
        'Categories',
      );
      cancelEdit();
    } catch (err) {
      const code = err instanceof ApiError ? err.code : undefined;
      editError =
        code === 'duplicate'
          ? 'A category with that name already exists.'
          : code === 'invalid_name'
            ? 'Name must be 1–100 characters.'
            : 'Could not rename. Please try again.';
    } finally {
      savingRename = false;
    }
  }

  function onEditKeydown(e: KeyboardEvent, cat: CategoryDto) {
    if (e.key === 'Enter') {
      e.preventDefault();
      void saveEdit(cat);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      cancelEdit();
    }
  }

  // ── Drag reorder ──────────────────────────────────────────────────────────
  function onDragStart(i: number, e: DragEvent) {
    draggedIdx = i;
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
    }
  }

  function onDragOver(i: number, e: DragEvent) {
    e.preventDefault();
    if (draggedIdx !== null) dragOverIdx = i;
  }

  function onDragEnd() {
    draggedIdx = null;
    dragOverIdx = null;
  }

  function onDrop(toIdx: number, e: DragEvent) {
    e.preventDefault();
    const from = draggedIdx;
    draggedIdx = null;
    dragOverIdx = null;
    if (from === null) return;
    void moveCategory(from, toIdx);
  }

  /** Shared by desktop drag & drop and the mobile up/down buttons. */
  async function moveCategory(from: number, to: number) {
    if (from === to || to < 0 || to >= localCategories.length) return;

    const arr = [...localCategories];
    const [item] = arr.splice(from, 1);
    arr.splice(to, 0, item);
    localCategories = arr;

    savingOrder = true;
    try {
      await reorderCategories(localCategories.map(c => c.id));
      await invalidateAll();
      notify.success('Category order saved.', 'Categories');
    } catch {
      notify.error('Failed to save category order.', 'Categories');
      localCategories = [...data.categories];
    } finally {
      savingOrder = false;
    }
  }

  const totalAssigned = $derived(
    localCategories.reduce((sum, c) => sum + (c.project_count ?? 0), 0),
  );
</script>

<div class="mt-8">
  <div class="mb-4 flex items-center justify-between">
    <div>
      <h2 class="font-heading text-[20px] font-semibold text-brand-text">Project Categories</h2>
      <p class="mt-0.5 text-sm text-brand-muted">
        {localCategories.length} {localCategories.length === 1 ? 'category' : 'categories'} defined,
        {totalAssigned} {totalAssigned === 1 ? 'project' : 'projects'} assigned.
        Drag to reorder — the order drives the projects sidebar.
      </p>
    </div>
  </div>

  <form
    method="POST"
    action="?/addCategory"
    use:enhance={() => {
      addingCategory = true;
      addCategoryError = undefined;
      return async ({ result, update }) => {
        await update({ reset: false });
        addingCategory = false;
        if (result.type === 'success' && result.data?.success) {
          newCategoryName = '';
          notify.success('Category added.', 'Categories');
        } else if (result.type === 'success') {
          const code = (result.data as { error?: string })?.error;
          if (code === 'duplicate') {
            addCategoryError = 'A category with that name already exists.';
          } else if (code === 'invalid_name') {
            addCategoryError = 'Name must be 1–100 characters.';
          } else {
            addCategoryError = 'An error occurred. Please try again.';
          }
        }
      };
    }}
    class="mb-4 flex items-start gap-3"
  >
    <div class="flex-1">
      <input
        type="text"
        name="name"
        placeholder="New category name…"
        bind:value={newCategoryName}
        maxlength={100}
        class="w-full rounded-[6px] border border-brand-border px-3 py-2 text-sm
               text-brand-text placeholder:text-brand-muted/60
               focus:outline-none focus:ring-2 focus:ring-brand-blue
               {addCategoryError ? 'border-red-400' : ''}"
      />
      {#if addCategoryError}
        <p class="mt-1 text-xs text-red-500">{addCategoryError}</p>
      {/if}
    </div>
    <button
      type="submit"
      disabled={addingCategory || !newCategoryName.trim()}
      class="rounded-btn bg-brand-blue px-5 py-2 text-sm font-semibold text-white
             transition-opacity hover:opacity-80 disabled:cursor-not-allowed disabled:opacity-40"
    >
      {addingCategory ? 'Adding…' : 'Add'}
    </button>
  </form>

  <div class="rounded-[8px] bg-white shadow-sm">
    {#if localCategories.length === 0}
      <div class="flex flex-col items-center justify-center py-16 text-brand-muted">
        <svg width="36" height="36" viewBox="0 0 24 24" fill="none" class="mb-3 opacity-40" aria-hidden="true">
          <path d="M3 7h18M3 12h18M3 17h18" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
        </svg>
        <p class="text-sm">No categories yet. Add one above.</p>
      </div>
    {:else}
      <!-- Cards below `xl`, like the sessions registry. HTML5 drag & drop
           never fires on touch, so the cards reorder with up/down buttons
           that call the same moveCategory the drop handler uses. -->
      <ul class="divide-y divide-brand-border/60 xl:hidden">
        {#each localCategories as cat, i (cat.id)}
          {@const inUse = (cat.project_count ?? 0) > 0}
          {@const isEditing = editingId === cat.id}
          <li class="flex flex-col gap-2.5 p-4">
            <div class="flex items-start justify-between gap-3">
              <div class="min-w-0 flex-1">
                {#if isEditing}
                  <input
                    type="text"
                    bind:value={editingName}
                    onkeydown={(e) => onEditKeydown(e, cat)}
                    maxlength={100}
                    disabled={savingRename}
                    aria-label="Category name"
                    class="w-full rounded-[6px] border px-2 py-1 text-sm
                           text-brand-text focus:outline-none focus:ring-2 focus:ring-brand-blue
                           disabled:opacity-50
                           {editError ? 'border-red-400' : 'border-brand-border'}"
                  />
                  {#if editError}
                    <p class="mt-1 text-xs text-red-500">{editError}</p>
                  {:else if inUse}
                    <p class="mt-1 text-xs text-brand-muted">
                      Renaming also updates {cat.project_count}
                      {cat.project_count === 1 ? 'project' : 'projects'}.
                    </p>
                  {/if}
                {:else}
                  <span class="text-sm font-medium text-brand-text">
                    <span class="mr-1.5 text-brand-muted">{i + 1}.</span>{cat.name}
                  </span>
                {/if}
              </div>
              {#if inUse}
                <span class="shrink-0 rounded-full bg-brand-blue/10 px-2 py-0.5 text-xs font-semibold text-brand-blue">
                  {cat.project_count} {cat.project_count === 1 ? 'project' : 'projects'}
                </span>
              {/if}
            </div>

            <div class="flex items-center justify-between gap-2">
              <div class="flex gap-1">
                <button
                  type="button"
                  onclick={() => moveCategory(i, i - 1)}
                  disabled={i === 0 || savingOrder || editingId !== null}
                  aria-label="Move {cat.name} up"
                  class="rounded border border-brand-border px-2.5 py-1 text-xs font-semibold text-brand-muted
                         transition-colors hover:text-brand-text disabled:opacity-40"
                >↑</button>
                <button
                  type="button"
                  onclick={() => moveCategory(i, i + 1)}
                  disabled={i === localCategories.length - 1 || savingOrder || editingId !== null}
                  aria-label="Move {cat.name} down"
                  class="rounded border border-brand-border px-2.5 py-1 text-xs font-semibold text-brand-muted
                         transition-colors hover:text-brand-text disabled:opacity-40"
                >↓</button>
              </div>
              {#if isEditing}
                <div class="flex items-center gap-2">
                  <button
                    type="button"
                    onclick={() => saveEdit(cat)}
                    disabled={savingRename || !editingName.trim()}
                    class="rounded-btn bg-brand-blue px-3 py-1 text-xs font-semibold text-white
                           transition-opacity hover:opacity-80 disabled:opacity-40"
                  >
                    {savingRename ? 'Saving…' : 'Save'}
                  </button>
                  <button
                    type="button"
                    onclick={cancelEdit}
                    disabled={savingRename}
                    class="rounded px-3 py-1 text-xs font-semibold text-brand-muted
                           transition-colors hover:text-brand-text disabled:opacity-40"
                  >
                    Cancel
                  </button>
                </div>
              {:else}
                <div class="flex items-center gap-1">
                  <button
                    type="button"
                    onclick={() => startEdit(cat)}
                    disabled={savingOrder || editingId !== null}
                    class="rounded px-3 py-1 text-xs font-semibold text-brand-blue
                           transition-colors hover:bg-brand-blue/10 disabled:opacity-40"
                  >
                    Edit
                  </button>
                  <form
                    method="POST"
                    action="?/deleteCategory"
                    use:enhance={({ cancel }) => {
                      // Deleting a category in use leaves those projects
                      // pointing at a category that no longer exists.
                      if (
                        inUse &&
                        !confirm(
                          `"${cat.name}" is used by ${cat.project_count} ` +
                            `${cat.project_count === 1 ? 'project' : 'projects'}. ` +
                            `Deleting it leaves ${cat.project_count === 1 ? 'that project' : 'those projects'} ` +
                            `without a valid category. Delete anyway?`,
                        )
                      ) {
                        cancel();
                        return;
                      }
                      deletingCategoryId = cat.id;
                      return async ({ result, update }) => {
                        await update();
                        deletingCategoryId = null;
                        if (result.type === 'success' && result.data?.success) {
                          notify.success(`"${cat.name}" deleted.`, 'Categories');
                        } else {
                          notify.error('Could not delete category.', 'Categories');
                        }
                      };
                    }}
                  >
                    <input type="hidden" name="id" value={cat.id} />
                    <button
                      type="submit"
                      disabled={deletingCategoryId === cat.id || savingOrder || editingId !== null}
                      class="rounded px-3 py-1 text-xs font-semibold text-red-500
                             transition-colors hover:bg-red-50 disabled:opacity-40"
                    >
                      {deletingCategoryId === cat.id ? 'Deleting…' : 'Delete'}
                    </button>
                  </form>
                </div>
              {/if}
            </div>
          </li>
        {/each}
      </ul>

      <!-- The drag-to-reorder table, at xl+ only where a mouse is likely. -->
      <div class="hidden overflow-x-auto xl:block">
      <table class="w-full">
        <thead>
          <tr class="border-b border-brand-border bg-brand-light-blue/40">
            <th class="w-10 px-4 py-3" aria-label="Drag handle"></th>
            <th class="px-6 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
              #
            </th>
            <th class="px-6 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
              Name
            </th>
            <th class="px-6 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
              Projects
            </th>
            <th class="px-6 py-3 text-right text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
              Actions
            </th>
          </tr>
        </thead>
        <tbody>
          {#each localCategories as cat, i (cat.id)}
            {@const inUse = (cat.project_count ?? 0) > 0}
            {@const isEditing = editingId === cat.id}
            <tr
              draggable={!isEditing}
              ondragstart={(e) => onDragStart(i, e)}
              ondragover={(e) => onDragOver(i, e)}
              ondragend={onDragEnd}
              ondrop={(e) => onDrop(i, e)}
              class="border-b border-brand-border/60 last:border-0 transition-colors
                     {dragOverIdx === i && draggedIdx !== null && draggedIdx !== i
                        ? 'bg-brand-light-blue/60 outline outline-2 outline-brand-blue/40'
                        : 'hover:bg-brand-light-blue/20'}
                     {draggedIdx === i ? 'opacity-40' : ''}"
            >
              <td
                class="w-10 px-4 py-4 text-center text-base leading-none
                       text-brand-muted/50 select-none
                       {isEditing ? '' : 'cursor-grab active:cursor-grabbing'}"
                aria-hidden="true"
              >⠿</td>
              <td class="w-12 px-6 py-4 text-sm text-brand-muted">{i + 1}</td>

              <td class="px-6 py-4 text-sm">
                {#if isEditing}
                  <input
                    type="text"
                    bind:value={editingName}
                    onkeydown={(e) => onEditKeydown(e, cat)}
                    maxlength={100}
                    disabled={savingRename}
                    aria-label="Category name"
                    class="w-full max-w-[280px] rounded-[6px] border px-2 py-1 text-sm
                           text-brand-text focus:outline-none focus:ring-2 focus:ring-brand-blue
                           disabled:opacity-50
                           {editError ? 'border-red-400' : 'border-brand-border'}"
                  />
                  {#if editError}
                    <p class="mt-1 text-xs text-red-500">{editError}</p>
                  {:else if inUse}
                    <p class="mt-1 text-xs text-brand-muted">
                      Renaming also updates {cat.project_count}
                      {cat.project_count === 1 ? 'project' : 'projects'}.
                    </p>
                  {/if}
                {:else}
                  <span class="font-medium text-brand-text">{cat.name}</span>
                {/if}
              </td>

              <td class="px-6 py-4 text-sm">
                {#if inUse}
                  <span class="rounded-full bg-brand-blue/10 px-2 py-0.5 text-xs font-semibold text-brand-blue">
                    {cat.project_count}
                  </span>
                {:else}
                  <span class="text-xs text-brand-muted">—</span>
                {/if}
              </td>

              <td class="px-6 py-4 text-right">
                {#if isEditing}
                  <div class="flex items-center justify-end gap-2">
                    <button
                      type="button"
                      onclick={() => saveEdit(cat)}
                      disabled={savingRename || !editingName.trim()}
                      class="rounded-btn bg-brand-blue px-3 py-1 text-xs font-semibold text-white
                             transition-opacity hover:opacity-80 disabled:opacity-40"
                    >
                      {savingRename ? 'Saving…' : 'Save'}
                    </button>
                    <button
                      type="button"
                      onclick={cancelEdit}
                      disabled={savingRename}
                      class="rounded px-3 py-1 text-xs font-semibold text-brand-muted
                             transition-colors hover:text-brand-text disabled:opacity-40"
                    >
                      Cancel
                    </button>
                  </div>
                {:else}
                  <div class="flex items-center justify-end gap-1">
                    <button
                      type="button"
                      onclick={() => startEdit(cat)}
                      disabled={savingOrder || editingId !== null}
                      class="rounded px-3 py-1 text-xs font-semibold text-brand-blue
                             transition-colors hover:bg-brand-blue/10 disabled:opacity-40"
                    >
                      Edit
                    </button>
                    <form
                      method="POST"
                      action="?/deleteCategory"
                      use:enhance={({ cancel }) => {
                        // Deleting a category in use leaves those projects
                        // pointing at a category that no longer exists.
                        if (
                          inUse &&
                          !confirm(
                            `"${cat.name}" is used by ${cat.project_count} ` +
                              `${cat.project_count === 1 ? 'project' : 'projects'}. ` +
                              `Deleting it leaves ${cat.project_count === 1 ? 'that project' : 'those projects'} ` +
                              `without a valid category. Delete anyway?`,
                          )
                        ) {
                          cancel();
                          return;
                        }
                        deletingCategoryId = cat.id;
                        return async ({ result, update }) => {
                          await update();
                          deletingCategoryId = null;
                          if (result.type === 'success' && result.data?.success) {
                            notify.success(`"${cat.name}" deleted.`, 'Categories');
                          } else {
                            notify.error('Could not delete category.', 'Categories');
                          }
                        };
                      }}
                    >
                      <input type="hidden" name="id" value={cat.id} />
                      <button
                        type="submit"
                        disabled={deletingCategoryId === cat.id || savingOrder || editingId !== null}
                        class="rounded px-3 py-1 text-xs font-semibold text-red-500
                               transition-colors hover:bg-red-50 disabled:opacity-40"
                      >
                        {deletingCategoryId === cat.id ? 'Deleting…' : 'Delete'}
                      </button>
                    </form>
                  </div>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
      </div>
      {#if savingOrder}
        <p class="border-t border-brand-border/40 px-6 py-2 text-xs text-brand-muted">
          Saving order…
        </p>
      {/if}
    {/if}
  </div>
</div>
