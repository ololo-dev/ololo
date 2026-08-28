<script lang="ts">
  interface Props {
    open?: boolean;
    deadline?: string;
    min?: string;
    increment?: string;
    max?: string;
    baseline?: string;
    idPrefix?: string;
  }

  let {
    open = $bindable(false),
    deadline = $bindable(''),
    min = $bindable(''),
    increment = $bindable(''),
    max = $bindable(''),
    baseline = '',
    idPrefix = '',
  }: Props = $props();

  const pid = (name: string) => (idPrefix ? `${idPrefix}-${name}` : name);
</script>

<div class="mb-4 mt-6 border-t border-brand-border pt-4">
  <button
    type="button"
    class="flex w-full items-center justify-between"
    onclick={() => (open = !open)}
  >
    <span class="text-xs font-semibold text-brand-text">
      Intervals defaults <span class="font-normal text-brand-muted">({baseline})</span>
    </span>
    <svg
      width="16" height="16" viewBox="0 0 24 24" fill="none"
      xmlns="http://www.w3.org/2000/svg"
      class="text-brand-muted transition-transform duration-200"
      style:transform={open ? 'rotate(180deg)' : 'rotate(90deg)'}
      aria-hidden="true"
    >
      <path d="M6 9l6 6 6-6" stroke="currentColor" stroke-width="2"
        stroke-linecap="round" stroke-linejoin="round" />
    </svg>
  </button>

  {#if open}
    <div class="mt-3 grid grid-cols-2 gap-3">
      <div>
        <label for={pid('intervals-deadline')} class="mb-1 block text-xs font-semibold text-brand-text">
          Deadline (secs)
        </label>
        <input
          id={pid('intervals-deadline')}
          name="intervals_deadline_secs"
          type="number"
          value={deadline}
          oninput={(e) => (deadline = (e.target as HTMLInputElement).value)}
          class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3
                 text-base text-brand-text placeholder:text-brand-muted
                 focus:border-brand-blue focus:outline-none"
        />
      </div>
      <div>
        <label for={pid('intervals-min')} class="mb-1 block text-xs font-semibold text-brand-text">
          Min interval (secs)
        </label>
        <input
          id={pid('intervals-min')}
          name="intervals_min_interval_secs"
          type="number"
          value={min}
          oninput={(e) => (min = (e.target as HTMLInputElement).value)}
          class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3
                 text-base text-brand-text placeholder:text-brand-muted
                 focus:border-brand-blue focus:outline-none"
        />
      </div>
      <div>
        <label for={pid('intervals-increment')} class="mb-1 block text-xs font-semibold text-brand-text">
          Increment (secs)
        </label>
        <input
          id={pid('intervals-increment')}
          name="intervals_interval_increment_secs"
          type="number"
          value={increment}
          oninput={(e) => (increment = (e.target as HTMLInputElement).value)}
          class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3
                 text-base text-brand-text placeholder:text-brand-muted
                 focus:border-brand-blue focus:outline-none"
        />
      </div>
      <div>
        <label for={pid('intervals-max')} class="mb-1 block text-xs font-semibold text-brand-text">
          Max interval (secs)
        </label>
        <input
          id={pid('intervals-max')}
          name="intervals_max_interval_secs"
          type="number"
          value={max}
          oninput={(e) => (max = (e.target as HTMLInputElement).value)}
          class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3
                 text-base text-brand-text placeholder:text-brand-muted
                 focus:border-brand-blue focus:outline-none"
        />
      </div>
    </div>
  {/if}
</div>
