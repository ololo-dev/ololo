<script lang="ts">
  import type { PlayerHistoryCommit } from '$lib/types/arena';
  import DiffView from './sessions/DiffView.svelte';

  type Props = {
    commits: PlayerHistoryCommit[];
    loading: boolean;
    error: string | null;
    onRefresh?: () => void;
    /** Optional sha → provenance label ("Task #2 · Parser", "Session start"). */
    labels?: Map<string, string>;
    title?: string;
  };

  let {
    commits,
    loading,
    error,
    onRefresh,
    labels = new Map(),
    title = 'Change history',
  }: Props = $props();

  let expandedCommits = $state(new Set<string>());

  function isCommitExpanded(sha: string): boolean {
    return expandedCommits.has(sha);
  }
  function toggleCommit(sha: string): void {
    const next = new Set(expandedCommits);
    if (next.has(sha)) next.delete(sha);
    else next.add(sha);
    expandedCommits = next;
  }

  function shortSha(sha: string): string {
    return sha.slice(0, 7);
  }
</script>

<div class="overflow-hidden rounded-[8px] bg-white shadow-sm">
  <div class="flex items-center justify-between border-b border-brand-border/60 px-6 py-4">
    <div class="flex items-baseline gap-3">
      <h2 class="font-heading text-[20px] font-semibold text-brand-text">{title}</h2>
      <span class="text-sm text-brand-muted">
        {commits.length} {commits.length === 1 ? 'commit' : 'commits'}
      </span>
    </div>
    {#if onRefresh}
      <button
        type="button"
        onclick={onRefresh}
        disabled={loading}
        class="inline-flex items-center gap-1.5 rounded-md border border-brand-border/60 px-3 py-1.5 text-xs font-semibold text-brand-muted transition-colors hover:bg-brand-light-blue/20 disabled:opacity-50"
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" aria-hidden="true" class={loading ? 'animate-spin' : ''}>
          <path d="M21 12a9 9 0 1 1-3-6.7M21 4v5h-5" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
        Refresh
      </button>
    {/if}
  </div>

  {#if loading}
    <div class="flex items-center justify-center py-12 text-sm text-brand-muted">
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" aria-hidden="true" class="mr-2 animate-spin">
        <path d="M21 12a9 9 0 1 1-3-6.7M21 4v5h-5" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
      </svg>
      Loading history…
    </div>
  {:else if error}
    <div class="px-6 py-10 text-center text-sm text-red-600">{error}</div>
  {:else if commits.length === 0}
    <div class="flex flex-col items-center justify-center py-12 text-brand-muted">
      <svg width="36" height="36" viewBox="0 0 24 24" fill="none" class="mb-3 opacity-40" aria-hidden="true">
        <path d="M12 8v4l3 3M21 12a9 9 0 1 1-6.2-8.6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
      </svg>
      <p class="text-sm">No commits pushed yet.</p>
    </div>
  {:else}
    <div class="divide-y divide-brand-border/60">
      {#each commits as commit (commit.sha)}
        {@const expanded = isCommitExpanded(commit.sha)}
        {@const visibleFiles = commit.files}
        <div>
          <button
            type="button"
            onclick={() => toggleCommit(commit.sha)}
            class="flex w-full items-start gap-3 px-6 py-4 text-left transition-colors hover:bg-brand-light-blue/20"
          >
            <svg
              width="14" height="14" viewBox="0 0 24 24" fill="none" aria-hidden="true"
              class="mt-1 shrink-0 text-brand-muted transition-transform {expanded ? 'rotate-90' : ''}"
            >
              <path d="M9 18l6-6-6-6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
            </svg>
            <div class="min-w-0 flex-1">
              <div class="mb-1 flex flex-wrap items-center gap-2">
                <code class="rounded bg-gray-100 px-1.5 py-0.5 font-mono text-[11px] text-brand-text/80">{shortSha(commit.sha)}</code>
                <span class="text-[13px] font-semibold text-brand-text">{commit.message.split('\n')[0]}</span>
                {#if labels.get(commit.sha)}
                  <span class="rounded-full bg-brand-blue/10 px-2 py-0.5 text-[10px] font-semibold text-brand-blue">{labels.get(commit.sha)}</span>
                {/if}
              </div>
              <div class="flex flex-wrap items-center gap-3 text-[11px] text-brand-muted">
                <span>{commit.author_name}</span>
                <span>&middot;</span>
                <time>{new Date(commit.author_time).toLocaleString()}</time>
                <span>&middot;</span>
                <span>{visibleFiles.length} {visibleFiles.length === 1 ? 'file' : 'files'}</span>
              </div>
            </div>
          </button>

          {#if expanded}
            <div class="border-t border-brand-border/40 bg-brand-light-blue/10 px-6 py-4 space-y-3">
              {#if commit.message.split('\n').length > 1}
                <pre class="whitespace-pre-wrap rounded-md bg-white px-3 py-2 text-[12px] text-brand-text/80 border border-brand-border/40">{commit.message.split('\n').slice(1).join('\n').trim()}</pre>
              {/if}
              {#if visibleFiles.length === 0}
                <p class="py-2 text-center text-sm text-brand-muted">No file changes in this commit.</p>
              {:else}
                <DiffView files={visibleFiles} />
              {/if}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>