<script lang="ts">
  import type { ComponentType } from 'svelte';
  import { browser } from '$app/environment';
  import MarkdownContent from './MarkdownContent.svelte';
  import {
    Heading1, Heading2, Heading3,
    Bold, Italic, Strikethrough,
    Code, FileCode,
    TextQuote, Link,
    List, ListOrdered, ListTodo,
    SeparatorHorizontal, Table,
    Eye, Pencil,
  } from 'lucide-svelte';

  let {
    value = $bindable(''),
    name,
    placeholder,
    ...rest
  }: {
    value?: string;
    name: string;
    placeholder?: string;
    [key: string]: unknown;
  } = $props();

  let textarea = $state<HTMLTextAreaElement | undefined>();
  let activeTab = $state<'write' | 'preview'>('write');

  // ── Formatting helpers ────────────────────────────────────────────────────

  /** Wraps the current selection (or a placeholder word) with before/after. */
  function wrap(before: string, after = before, placeholder = 'text') {
    if (!textarea) return;
    const s = textarea.selectionStart;
    const e = textarea.selectionEnd;
    const selected = value.slice(s, e) || placeholder;
    value = value.slice(0, s) + before + selected + after + value.slice(e);
    requestAnimationFrame(() => {
      textarea!.focus();
      textarea!.setSelectionRange(s + before.length, s + before.length + selected.length);
    });
  }

  /** Inserts prefix at the start of the current line. */
  function prependLine(prefix: string, placeholder = 'item') {
    if (!textarea) return;
    const s = textarea.selectionStart;
    const e = textarea.selectionEnd;
    const lineStart = value.lastIndexOf('\n', s - 1) + 1;
    value = value.slice(0, lineStart) + prefix + value.slice(lineStart);
    requestAnimationFrame(() => {
      textarea!.focus();
      textarea!.setSelectionRange(
        s + prefix.length,
        s === e ? s + prefix.length : e + prefix.length,
      );
    });
  }

  /** Inserts a fenced block after the current cursor position. */
  function insertBlock(content: string) {
    if (!textarea) return;
    const s = textarea.selectionStart;
    const before = value.slice(0, s);
    const after = value.slice(s);
    const nl1 = before.length > 0 && !before.endsWith('\n') ? '\n' : '';
    const nl2 = after.length > 0 && !after.startsWith('\n') ? '\n' : '';
    value = before + nl1 + content + nl2 + after;
    const newPos = before.length + nl1.length + content.length + nl2.length;
    requestAnimationFrame(() => {
      textarea!.focus();
      textarea!.setSelectionRange(newPos, newPos);
    });
  }

  function insertLink() {
    if (!textarea) return;
    const s = textarea.selectionStart;
    const e = textarea.selectionEnd;
    const selected = value.slice(s, e) || 'link text';
    value = value.slice(0, s) + `[${selected}](url)` + value.slice(e);
    // Select the "url" placeholder so user can type over it immediately
    const urlStart = s + selected.length + 3;
    requestAnimationFrame(() => {
      textarea!.focus();
      textarea!.setSelectionRange(urlStart, urlStart + 3);
    });
  }

  // ── Toolbar button groups ─────────────────────────────────────────────────

  type Btn = { icon: ComponentType; title: string; action: () => void };

  const toolbar: Btn[][] = [
    [
      { icon: Heading1, title: 'Heading 1',  action: () => prependLine('# ',   'Heading') },
      { icon: Heading2, title: 'Heading 2',  action: () => prependLine('## ',  'Heading') },
      { icon: Heading3, title: 'Heading 3',  action: () => prependLine('### ', 'Heading') },
    ],
    [
      { icon: Bold,          title: 'Bold',          action: () => wrap('**', '**', 'bold text')   },
      { icon: Italic,        title: 'Italic',        action: () => wrap('_',  '_',  'italic text') },
      { icon: Strikethrough, title: 'Strikethrough', action: () => wrap('~~', '~~', 'text')        },
      { icon: Code,          title: 'Inline code',   action: () => wrap('`',  '`',  'code')        },
    ],
    [
      { icon: FileCode,  title: 'Code block',  action: () => insertBlock('```\ncode\n```')  },
      { icon: TextQuote, title: 'Blockquote',  action: () => prependLine('> ', 'quote')     },
      { icon: Link,      title: 'Link',        action: () => insertLink()                   },
    ],
    [
      { icon: List,              title: 'Unordered list', action: () => prependLine('- ')       },
      { icon: ListOrdered,       title: 'Ordered list',   action: () => prependLine('1. ')      },
      { icon: ListTodo,          title: 'Task list',      action: () => prependLine('- [ ] ')   },
    ],
    [
      { icon: SeparatorHorizontal, title: 'Horizontal rule', action: () => insertBlock('---')                                              },
      { icon: Table,               title: 'Table',           action: () => insertBlock('| Header | Header |\n| --- | --- |\n| Cell | Cell |') },
    ],
  ];
</script>

{#if browser}
  <div class="mt-1 overflow-hidden rounded border border-slate-300 focus-within:border-slate-500 focus-within:ring-1 focus-within:ring-slate-300">
    <!-- Tab bar + toolbar -->
    <div class="flex items-stretch border-b border-slate-200 bg-slate-50">
      <!-- Write / Preview tabs -->
      <div class="flex shrink-0 border-r border-slate-200">
        <button
          type="button"
          onclick={() => { activeTab = 'write'; }}
          class="flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium transition-colors
                 {activeTab === 'write' ? 'bg-white text-slate-900' : 'text-slate-500 hover:text-slate-700'}"
          title="Write"
        >
          <Pencil size={13} />
          Write
        </button>
        <button
          type="button"
          onclick={() => { activeTab = 'preview'; }}
          class="flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium transition-colors
                 {activeTab === 'preview' ? 'bg-white text-slate-900' : 'text-slate-500 hover:text-slate-700'}"
          title="Preview"
        >
          <Eye size={13} />
          Preview
        </button>
      </div>

      <!-- Toolbar (write mode only) -->
      {#if activeTab === 'write'}
        <div class="flex min-w-0 flex-wrap items-center gap-0 px-1 py-0.5">
          {#each toolbar as group, gi}
            {#if gi > 0}
              <span class="mx-1 h-4 w-px bg-slate-300" aria-hidden="true"></span>
            {/if}
            {#each group as btn}
              {@const Icon = btn.icon}
              <button
                type="button"
                title={btn.title}
                onclick={btn.action}
                class="rounded p-1 text-slate-500 transition-colors hover:bg-slate-200 hover:text-slate-900"
              >
                <Icon size={14} />
              </button>
            {/each}
          {/each}
        </div>
      {/if}
    </div>

    <!-- Write area (always in DOM so bind:this and formatting work; hidden in preview) -->
    <textarea
      bind:this={textarea}
      bind:value
      {placeholder}
      rows={6}
      class="w-full resize-y px-3 py-2 font-mono text-sm focus:outline-none
             {activeTab === 'preview' ? 'hidden' : ''}"
    ></textarea>

    <!-- Preview area -->
    {#if activeTab === 'preview'}
      <div class="min-h-[9rem] px-3 py-2">
        {#if value.trim()}
          <div class="prose prose-sm max-w-none">
            <MarkdownContent {value} />
          </div>
        {:else}
          <p class="text-sm italic text-slate-400">Nothing to preview.</p>
        {/if}
      </div>
    {/if}
  </div>

  <!-- Hidden input: authoritative form-submission field in browser mode -->
  <input type="hidden" {name} {value} />
  <!-- Visually-hidden textarea keeps data-testid and other rest props queryable in tests -->
  <textarea style="display:none" {name} {value} {...rest}></textarea>
{:else}
  <textarea {name} bind:value {placeholder} {...rest}></textarea>
{/if}
