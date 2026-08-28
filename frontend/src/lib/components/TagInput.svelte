<script lang="ts">
  interface Props {
    tags?: string[];
    onchange?: (tags: string[]) => void;
  }

  let { tags = [], onchange }: Props = $props();

  let inputValue = $state('');

  function addTag() {
    const trimmed = inputValue.trim().replace(/,$/, '').trim();
    if (trimmed && trimmed.length <= 50 && tags.length < 10 && !tags.includes(trimmed)) {
      onchange?.([...tags, trimmed]);
    }
    inputValue = '';
  }

  function removeTag(index: number) {
    onchange?.(tags.filter((_, i) => i !== index));
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' || e.key === ',') {
      e.preventDefault();
      addTag();
    }
  }

  function handleInput(e: Event) {
    inputValue = (e.target as HTMLInputElement).value;
    if (inputValue.endsWith(',')) {
      addTag();
    }
  }
</script>

<div class="flex flex-wrap gap-1.5 rounded-btn border border-brand-border bg-brand-light-blue/10 p-2 font-body focus-within:border-brand-blue">
  {#each tags as tag, i}
    <span class="flex items-center gap-1 rounded-full border border-brand-border bg-brand-light-blue px-2 py-0.5 text-sm text-brand-text">
      {tag}
      <button
        type="button"
        class="ml-0.5 text-brand-muted hover:text-brand-red focus:outline-none"
        onclick={() => removeTag(i)}
        aria-label="Remove tag {tag}"
      >×</button>
    </span>
  {/each}

  {#if tags.length < 10}
    <input
      type="text"
      class="min-w-[120px] flex-1 bg-transparent text-sm text-brand-text placeholder:text-brand-muted focus:outline-none"
      placeholder={tags.length === 0 ? 'Add tags (Enter or comma)' : 'Add tag…'}
      value={inputValue}
      oninput={handleInput}
      onkeydown={handleKeydown}
    />
  {/if}
</div>
