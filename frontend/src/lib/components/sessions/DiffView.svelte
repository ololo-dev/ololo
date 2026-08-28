<script lang="ts">
  import type { PlayerHistoryFileDiff } from '$lib/types/arena';

  type Props = { files: PlayerHistoryFileDiff[] };
  let { files }: Props = $props();

  const visibleFiles = $derived(files);

  let openFiles = $state(new Set<string>());

  function toggleFile(path: string): void {
    const next = new Set(openFiles);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    openFiles = next;
  }

  type DiffLine = { kind: 'context' | 'add' | 'remove' | 'meta'; text: string };

  function parsePatch(patch: string): DiffLine[] {
    const lines: DiffLine[] = [];
    for (const raw of patch.split('\n')) {
      if (raw.startsWith('@@') || raw.startsWith('diff --git') || raw.startsWith('index ') || raw.startsWith('---') || raw.startsWith('+++')) {
        lines.push({ kind: 'meta', text: raw });
      } else if (raw.startsWith('+')) {
        lines.push({ kind: 'add', text: raw });
      } else if (raw.startsWith('-')) {
        lines.push({ kind: 'remove', text: raw });
      } else {
        lines.push({ kind: 'context', text: raw });
      }
    }
    return lines;
  }

  function lineClass(line: DiffLine): string {
    switch (line.kind) {
      case 'add':    return 'bg-green-50 text-green-800';
      case 'remove': return 'bg-red-50 text-red-800';
      case 'meta':   return 'text-brand-blue/70 bg-brand-blue/5';
      default:       return 'text-brand-text/70';
    }
  }

  function fileBadgeClass(status: string): string {
    switch (status) {
      case 'added':    return 'bg-green-100 text-green-700';
      case 'modified': return 'bg-amber-100 text-amber-700';
      case 'deleted':  return 'bg-red-100 text-red-600';
      default:         return 'bg-gray-100 text-gray-500';
    }
  }
</script>

<div class="space-y-3" data-testid="diff-view">
  {#each visibleFiles as file (file.path)}
    {@const expanded = openFiles.has(file.path)}
    <div class="rounded-md border border-brand-border/60 bg-white overflow-hidden">
      <button
        type="button"
        onclick={() => toggleFile(file.path)}
        class="flex w-full items-center gap-2 px-4 py-2.5 border-b border-brand-border/40 cursor-pointer hover:bg-brand-light-blue/20"
      >
        <svg width="10" height="10" viewBox="0 0 24 24" fill="none" aria-hidden="true" class="shrink-0 text-brand-muted transition-transform {expanded ? 'rotate-90' : ''}"><path d="M9 18l6-6-6-6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>
        <span class="inline-flex items-center rounded-full px-2 py-0.5 text-[10px] font-semibold {fileBadgeClass(file.status)}">{file.status}</span>
        <code class="font-mono text-[12px] text-brand-text">{file.path}</code>
      </button>
      {#if expanded}
        <div class="overflow-x-auto bg-gray-50">
          <pre class="text-[11px] leading-relaxed font-mono"><code>{#each parsePatch(file.patch) as line, i (i)}<span class="block px-2 {lineClass(line)}">{line.text || ' '}</span>{/each}</code></pre>
        </div>
      {/if}
    </div>
  {/each}
</div>