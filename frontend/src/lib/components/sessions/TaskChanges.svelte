<script lang="ts">
  import type { PlayerHistoryCommit } from '$lib/types/arena';
  import DiffView from './DiffView.svelte';
  import { reduceDiff } from '$lib/sessions/diff-reduce';

  type Props = {
    commits: PlayerHistoryCommit[];
    mode: 'range' | 'per-commit';
    testid: string;
  };
  let { commits, mode, testid }: Props = $props();

  const rangeFiles = $derived(mode === 'range' ? reduceDiff(commits) : []);
</script>

<div data-testid={testid}>
  {#if commits.length === 0}
    <p class="py-6 text-center text-sm text-brand-muted">No changes attributed to this task.</p>
  {:else if mode === 'range'}
    <DiffView files={rangeFiles} />
  {:else}
    <div class="space-y-4">
      {#each commits as commit (commit.sha)}
        <div class="rounded-md border border-brand-border/40 overflow-hidden">
          <div class="flex items-center gap-2 px-4 py-2 bg-white border-b border-brand-border/40">
            <code class="font-mono text-[11px] text-brand-text/80">{commit.sha.slice(0, 7)}</code>
            <span class="text-[12px] font-semibold text-brand-text">{commit.message.split('\n')[0]}</span>
            <span class="ml-auto text-[11px] text-brand-muted">{new Date(commit.author_time).toLocaleString()}</span>
          </div>
          {#if commit.files.length > 0}
            <DiffView files={commit.files} />
          {:else}
            <p class="px-4 py-3 text-[12px] text-brand-muted">No file changes in this commit.</p>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>
