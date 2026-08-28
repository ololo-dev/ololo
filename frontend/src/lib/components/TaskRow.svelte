<script lang="ts">
  import { GripVertical, ArrowUp, ArrowDown, Pencil, Trash2 } from 'lucide-svelte';
  import { enhance } from '$app/forms';
  import type { Task } from '$lib/api';

  let {
    task,
    index,
    total,
    draggedId,
    dragOverId,
    onMove,
    onEdit,
    onDragStart,
    onDragEnd,
    onDragOver,
    onDragLeave,
    onDrop,
  }: {
    task: Task;
    index: number;
    total: number;
    draggedId: string | null;
    dragOverId: string | null;
    onMove: (id: string, direction: 'up' | 'down') => void;
    onEdit: (id: string) => void;
    onDragStart: (id: string) => void;
    onDragEnd: () => void;
    onDragOver: (id: string) => void;
    onDragLeave: (id: string) => void;
    onDrop: (id: string) => void;
  } = $props();
</script>

<li
  class="grid grid-cols-[2rem_3rem_1fr_10rem_auto] items-center gap-4
         border-b border-brand-border px-5 py-3 last:border-0
         transition-colors hover:bg-brand-light-blue
         {draggedId === task.id ? 'opacity-40' : ''}
         {dragOverId === task.id && draggedId !== task.id ? 'border-t-2 border-t-brand-blue' : ''}"
  ondragstart={() => onDragStart(task.id)}
  ondragend={(e) => { onDragEnd(); (e.currentTarget as HTMLElement).setAttribute('draggable', 'false'); }}
  ondragover={(e) => { e.preventDefault(); onDragOver(task.id); }}
  ondragleave={(e) => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) onDragLeave(task.id); }}
  ondrop={(e) => { e.preventDefault(); onDrop(task.id); onDragEnd(); }}
>
  <!-- Drag handle -->
  <div
    class="cursor-grab active:cursor-grabbing text-brand-muted"
    aria-hidden="true"
    onmousedown={(e) => { (e.currentTarget as HTMLElement).closest('li')?.setAttribute('draggable', 'true'); }}
    onmouseup={(e) => { (e.currentTarget as HTMLElement).closest('li')?.setAttribute('draggable', 'false'); }}
  ><GripVertical size={16} /></div>

  <!-- Ordinal -->
  <div class="text-sm text-brand-muted">{task.ordinal}</div>

  <!-- Title + description excerpt -->
  <div class="min-w-0">
    <div class="truncate text-sm font-semibold text-brand-text">{task.title}</div>
    {#if task.description}
      <div class="mt-0.5 truncate text-xs text-brand-muted">
        {task.description.slice(0, 120)}
      </div>
    {/if}
  </div>

  <!-- Tags -->
  <div
    class="flex min-w-0 flex-wrap gap-1"
    data-testid={task.tags && task.tags.length > 0 ? `task-tags-${task.id}` : undefined}
  >
    {#if task.tags}
      {#each task.tags as tag}
        <span class="inline-block rounded-[4px] bg-brand-light-blue px-1.5 py-0.5 text-xs text-brand-muted">
          {tag}
        </span>
      {/each}
    {/if}
  </div>

  <!-- Actions -->
  <div class="flex items-center gap-1">
    <button
      type="button"
      onclick={() => onMove(task.id, 'up')}
      disabled={index === 0}
      class="rounded p-1 text-brand-muted transition-colors hover:bg-brand-light-blue hover:text-brand-text disabled:opacity-25"
      aria-label="Move task up"
      data-testid="task-move-up"
    ><ArrowUp size={14} /></button>

    <button
      type="button"
      onclick={() => onMove(task.id, 'down')}
      disabled={index === total - 1}
      class="rounded p-1 text-brand-muted transition-colors hover:bg-brand-light-blue hover:text-brand-text disabled:opacity-25"
      aria-label="Move task down"
      data-testid="task-move-down"
    ><ArrowDown size={14} /></button>

    <button
      type="button"
      onclick={() => onEdit(task.id)}
      class="flex items-center gap-1 rounded-[4px] border border-brand-border px-2 py-1
             text-xs text-brand-muted transition-colors hover:border-brand-blue hover:text-brand-blue"
      data-testid="edit-task-btn-{task.id}"
    >
      <Pencil size={11} /> Edit
    </button>

    <form method="POST" action="?/deleteTask" use:enhance>
      <input type="hidden" name="task_id" value={task.id} />
      <button
        type="submit"
        class="rounded-[4px] p-1 text-brand-muted transition-colors hover:bg-red-50 hover:text-brand-red"
        aria-label="Delete task"
        data-testid="delete-task"
      ><Trash2 size={14} /></button>
    </form>
  </div>
</li>
