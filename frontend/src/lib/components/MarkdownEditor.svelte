<!--
  A markdown textarea with a formatting toolbar and a rendered preview.

  Deliberately not a rich-text editor: the value is plain markdown, the
  textarea stays the form control (`name` submits as usual), and the toolbar
  only splices markers around the current selection. Preview renders through
  MarkdownContent — the same pipeline the public pages use, so what the admin
  sees is what the site serves.
-->
<script lang="ts">
  import { tick } from 'svelte';
  import MarkdownContent from '$lib/components/MarkdownContent.svelte';

  interface Props {
    value: string;
    /** One-way + callback, per house convention (bind:prop breaks SSR). */
    oninput: (value: string) => void;
    name?: string;
    id?: string;
    rows?: number;
    placeholder?: string;
  }

  let { value, oninput, name, id, rows = 22, placeholder }: Props = $props();

  let textareaEl = $state<HTMLTextAreaElement | null>(null);
  let mode = $state<'write' | 'preview'>('write');

  /** Wrap the selection in `before`/`after`, or drop a placeholder there. */
  async function wrapSelection(before: string, after: string, fallback: string) {
    const el = textareaEl;
    if (!el) return;
    const start = el.selectionStart;
    const end = el.selectionEnd;
    const inner = value.slice(start, end) || fallback;
    const next = value.slice(0, start) + before + inner + after + value.slice(end);
    oninput(next);
    await tick();
    el.focus();
    el.setSelectionRange(start + before.length, start + before.length + inner.length);
  }

  /** Prefix every selected line — headings, lists, quotes. */
  async function prefixLines(prefix: string | ((i: number) => string)) {
    const el = textareaEl;
    if (!el) return;
    const start = el.selectionStart;
    const end = el.selectionEnd;
    const lineStart = value.lastIndexOf('\n', start - 1) + 1;
    const tail = value.indexOf('\n', end);
    const lineEnd = tail === -1 ? value.length : tail;
    const block = value
      .slice(lineStart, lineEnd)
      .split('\n')
      .map((line, i) => (typeof prefix === 'string' ? prefix : prefix(i)) + line)
      .join('\n');
    const next = value.slice(0, lineStart) + block + value.slice(lineEnd);
    oninput(next);
    await tick();
    el.focus();
    el.setSelectionRange(lineStart, lineStart + block.length);
  }

  const MODES = ['write', 'preview'] as const;

  const TOOLS: { label: string; title: string; run: () => void }[] = [
    { label: 'B', title: 'Bold', run: () => wrapSelection('**', '**', 'bold') },
    { label: 'I', title: 'Italic', run: () => wrapSelection('*', '*', 'italic') },
    { label: 'H2', title: 'Heading', run: () => prefixLines('## ') },
    { label: '🔗', title: 'Link', run: () => wrapSelection('[', '](https://)', 'link text') },
    { label: '•', title: 'Bullet list', run: () => prefixLines('- ') },
    { label: '1.', title: 'Numbered list', run: () => prefixLines((i) => `${i + 1}. `) },
    { label: '❝', title: 'Quote', run: () => prefixLines('> ') },
    { label: '</>', title: 'Code', run: () => wrapSelection('`', '`', 'code') },
  ];
</script>

<div class="rounded-[6px] border border-brand-border">
  <div class="flex flex-wrap items-center gap-1 border-b border-brand-border bg-brand-light-blue/40 px-2 py-1.5">
    <div class="flex items-center gap-0.5" role="toolbar" aria-label="Formatting">
      {#each TOOLS as tool (tool.title)}
        <button
          type="button"
          title={tool.title}
          aria-label={tool.title}
          onclick={tool.run}
          disabled={mode === 'preview'}
          class="rounded px-2 py-1 font-mono text-xs font-semibold text-brand-muted
                 transition-colors hover:bg-brand-blue/10 hover:text-brand-text
                 disabled:pointer-events-none disabled:opacity-40"
        >{tool.label}</button>
      {/each}
    </div>
    <div class="ml-auto flex items-center gap-1">
      {#each MODES as m (m)}
        <button
          type="button"
          onclick={() => (mode = m)}
          aria-pressed={mode === m}
          data-testid="md-{m}"
          class="rounded px-2.5 py-1 text-xs font-semibold capitalize transition-colors
                 {mode === m ? 'bg-brand-blue text-white' : 'text-brand-muted hover:bg-brand-blue/10'}"
        >{m}</button>
      {/each}
    </div>
  </div>

  <!-- The textarea is hidden, not removed, in preview: it is the form control. -->
  <textarea
    bind:this={textareaEl}
    {id}
    {name}
    {rows}
    {placeholder}
    {value}
    oninput={(e) => oninput((e.target as HTMLTextAreaElement).value)}
    class="w-full rounded-b-[6px] border-0 px-3 py-2 font-mono text-xs leading-relaxed
           focus:outline-none focus:ring-2 focus:ring-brand-blue
           {mode === 'preview' ? 'hidden' : ''}"
  ></textarea>
  {#if mode === 'preview'}
    <div class="max-h-[520px] overflow-y-auto px-4 py-3" data-testid="md-preview-pane">
      {#if value.trim()}
        <MarkdownContent value={value} />
      {:else}
        <p class="text-sm text-brand-muted">Nothing to preview.</p>
      {/if}
    </div>
  {/if}
</div>
