<script lang="ts">
  interface Props {
    open?: boolean;
    value?: string;
    fail?: string;
    noResponse?: string;
    completionBonus?: string;
    baseline?: string;
    idPrefix?: string;
  }

  let {
    open = $bindable(false),
    value = $bindable(''),
    fail = $bindable(''),
    noResponse = $bindable(''),
    completionBonus = $bindable(''),
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
      Points defaults <span class="font-normal text-brand-muted">({baseline})</span>
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
        <label for={pid('points-value')} class="mb-1 block text-xs font-semibold text-brand-text">
          Point value <span class="font-normal text-brand-muted">(display + pass score)</span>
        </label>
        <input
          id={pid('points-value')}
          name="points_value"
          type="number"
          value={value}
          oninput={(e) => (value = (e.target as HTMLInputElement).value)}
          class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3
                 text-base text-brand-text placeholder:text-brand-muted
                 focus:border-brand-blue focus:outline-none"
        />
      </div>
      <div>
        <label for={pid('points-fail')} class="mb-1 block text-xs font-semibold text-brand-text">
          Fail points
        </label>
        <input
          id={pid('points-fail')}
          name="points_fail"
          type="number"
          value={fail}
          oninput={(e) => (fail = (e.target as HTMLInputElement).value)}
          class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3
                 text-base text-brand-text placeholder:text-brand-muted
                 focus:border-brand-blue focus:outline-none"
        />
      </div>
      <div>
        <label for={pid('points-no-response')} class="mb-1 block text-xs font-semibold text-brand-text">
          No-response points
        </label>
        <input
          id={pid('points-no-response')}
          name="points_no_response"
          type="number"
          value={noResponse}
          oninput={(e) => (noResponse = (e.target as HTMLInputElement).value)}
          class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3
                 text-base text-brand-text placeholder:text-brand-muted
                 focus:border-brand-blue focus:outline-none"
        />
      </div>
      <div>
        <label for={pid('points-completion-bonus')} class="mb-1 block text-xs font-semibold text-brand-text">
          Completion bonus
        </label>
        <input
          id={pid('points-completion-bonus')}
          name="points_completion_bonus"
          type="number"
          value={completionBonus}
          oninput={(e) => (completionBonus = (e.target as HTMLInputElement).value)}
          class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3
                 text-base text-brand-text placeholder:text-brand-muted
                 focus:border-brand-blue focus:outline-none"
        />
      </div>
    </div>
  {/if}
</div>
