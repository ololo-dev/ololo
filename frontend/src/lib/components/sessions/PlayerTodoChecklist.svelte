<script lang="ts">
  import type { PlayerTaskEvaluation } from '$lib/types/arena'

  let { evaluation }: { evaluation: PlayerTaskEvaluation } = $props()

  const todo = $derived(evaluation.todo ?? null)
  const pct = $derived(
    todo && todo.total > 0 ? Math.round((todo.checked / todo.total) * 100) : 0,
  )
</script>

<!-- The plan reads as a document section, not a widget: no box, a quiet
     section label, one thin progress line, and the items in comfortable
     type. Long plans flow into two columns on wide screens. -->
{#if todo && todo.total > 0}
  <div data-testid="todo-checklist">
    <div class="mb-2 flex items-baseline justify-between gap-3">
      <span class="text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Plan</span>
      <span class="text-[13px] font-semibold tabular-nums {pct === 100 ? 'text-green-600' : 'text-brand-muted'}">
        {todo.checked}/{todo.total} done
      </span>
    </div>
    <div class="mb-4 h-1 overflow-hidden rounded bg-brand-border/30">
      <div
        class="h-full rounded transition-all {pct === 100 ? 'bg-green-500' : 'bg-brand-blue'}"
        style="width: {pct}%"
      ></div>
    </div>
    <ul class="gap-x-10 space-y-2 {todo.items.length > 8 ? 'lg:columns-2' : ''}">
      {#each todo.items as item, i (i)}
        <li class="flex items-start gap-2.5 break-inside-avoid text-[14px] leading-snug">
          <span
            class="mt-[3px] inline-flex h-4 w-4 shrink-0 items-center justify-center rounded-full {item.done
              ? 'bg-green-500 text-white'
              : 'border-[1.5px] border-brand-border'}"
            aria-hidden="true"
          >
            {#if item.done}<svg viewBox="0 0 10 10" class="h-2.5 w-2.5 fill-none stroke-current stroke-2"><path d="M1.5 5.5l2.5 2.5 4.5-5.5" /></svg>{/if}
          </span>
          <!-- Untrusted player-authored text: rendered as text, never HTML. -->
          <span class={item.done ? 'text-brand-muted' : 'text-brand-text'}>
            {item.text}
          </span>
        </li>
      {/each}
    </ul>
  </div>
{/if}
