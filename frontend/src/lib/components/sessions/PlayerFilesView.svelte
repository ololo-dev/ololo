<script lang="ts">
  import { untrack } from 'svelte';
  import type { PlayerFileContent, PlayerFilesResponse } from '$lib/types/arena';
  import { highlightToHtml } from '$lib/highlight';
  import {
    ancestorDirs,
    buildFileTree,
    defaultFile,
    formatBytes,
    type FileTreeNode,
  } from '$lib/sessions/file-tree';

  type Props = {
    sessionCode: string;
    playerId: string;
    /**
     * The newest commit the page knows of. The tree is re-read whenever it
     * changes, so a push that lands while the tab is open shows up.
     */
    revision: string | null;
  };
  let { sessionCode, playerId, revision }: Props = $props();

  const base = $derived(
    `/api/sessions/${encodeURIComponent(sessionCode)}/players/${encodeURIComponent(playerId)}/files`,
  );
  const IMAGE_EXT = /\.(png|jpe?g|webp|gif)$/i;

  let listing = $state<PlayerFilesResponse | null>(null);
  let treeLoading = $state(false);
  let treeError = $state<string | null>(null);
  let selected = $state<string | null>(null);
  let file = $state<PlayerFileContent | null>(null);
  let fileLoading = $state(false);
  let fileError = $state<string | null>(null);
  let open = $state(new Set<string>());

  const tree = $derived(listing ? buildFileTree(listing.files) : []);
  const lineNumbers = $derived.by(() => {
    const text = file?.content ?? '';
    const count = text.split('\n').length - (text.endsWith('\n') ? 1 : 0);
    return Array.from({ length: Math.max(count, 1) }, (_, i) => i + 1).join('\n');
  });

  async function loadTree(): Promise<void> {
    treeLoading = true;
    treeError = null;
    try {
      const resp = await fetch(base);
      if (!resp.ok) {
        treeError = `Failed to load files (HTTP ${resp.status})`;
        return;
      }
      const body = (await resp.json()) as PlayerFilesResponse;
      listing = body;
      // Small repos open fully; big ones open just the way to the file.
      if (open.size === 0 && body.files.length <= 100) {
        open = new Set(body.files.flatMap((f) => ancestorDirs(f.path)));
      }
      const keep = selected && body.files.some((f) => f.path === selected);
      void select(keep ? selected : defaultFile(body.files));
    } catch (e) {
      treeError = `Failed to load files: ${e instanceof Error ? e.message : String(e)}`;
    } finally {
      treeLoading = false;
    }
  }

  // Responses can arrive out of order when the viewer clicks quickly;
  // only the latest request may write the panel.
  let fileRequest = 0;

  async function select(path: string | null): Promise<void> {
    selected = path;
    file = null;
    fileError = null;
    if (!path || !listing?.head) return;
    open = new Set([...open, ...ancestorDirs(path)]);
    const ticket = ++fileRequest;
    fileLoading = true;
    try {
      const qs = new URLSearchParams({ path, sha: listing.head.sha });
      const resp = await fetch(`${base}/content?${qs}`);
      if (ticket !== fileRequest) return;
      if (!resp.ok) {
        fileError = `Failed to load ${path} (HTTP ${resp.status})`;
        return;
      }
      file = (await resp.json()) as PlayerFileContent;
    } catch (e) {
      if (ticket === fileRequest) {
        fileError = `Failed to load ${path}: ${e instanceof Error ? e.message : String(e)}`;
      }
    } finally {
      if (ticket === fileRequest) fileLoading = false;
    }
  }

  function toggleDir(path: string): void {
    const next = new Set(open);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    open = next;
  }

  $effect(() => {
    void revision;
    untrack(() => void loadTree());
  });

  // Plain text shows at once; the colours swap in when shiki and the
  // file's grammar have loaded. Tagged with the file they belong to, so a
  // slow highlight never lands on the file the viewer moved on to.
  let highlighted = $state.raw<{ file: PlayerFileContent; html: string } | null>(null);
  const highlightedHtml = $derived(highlighted?.file === file ? highlighted.html : null);
  $effect(() => {
    const current = file;
    if (!current?.content) return;
    let live = true;
    void highlightToHtml(current.content, current.path).then((html) => {
      if (live && html) highlighted = { file: current, html };
    });
    return () => {
      live = false;
    };
  });
</script>

{#snippet nodes(list: FileTreeNode[], depth: number)}
  {#each list as node (node.path)}
    {#if node.kind === 'dir'}
      <button
        type="button"
        class="flex w-full items-center gap-1.5 py-1 pr-2 text-left text-[12px] text-brand-text hover:bg-brand-blue/5"
        style="padding-left: {8 + depth * 14}px"
        aria-expanded={open.has(node.path)}
        onclick={() => toggleDir(node.path)}
      >
        <span class="w-3 shrink-0 text-[10px] text-brand-muted">{open.has(node.path) ? '▾' : '▸'}</span>
        <span class="truncate font-semibold">{node.name}/</span>
      </button>
      {#if open.has(node.path)}
        {@render nodes(node.children, depth + 1)}
      {/if}
    {:else}
      <button
        type="button"
        data-testid="file-{node.path}"
        aria-current={selected === node.path ? 'true' : undefined}
        class="flex w-full items-center gap-1.5 py-1 pr-2 text-left font-mono text-[12px] transition-colors
               {selected === node.path
          ? 'bg-brand-blue/10 font-semibold text-brand-blue'
          : 'text-brand-text hover:bg-brand-blue/5'}"
        style="padding-left: {20 + depth * 14}px"
        onclick={() => void select(node.path)}
      >
        <span class="truncate">{node.name}</span>
      </button>
    {/if}
  {/each}
{/snippet}

<div data-testid="player-files-view">
  {#if treeError}
    <div class="rounded-[8px] bg-white py-16 text-center text-sm text-red-600 shadow-sm">
      {treeError}
    </div>
  {:else if !listing}
    <div class="rounded-[8px] bg-white py-16 text-center text-sm text-brand-muted shadow-sm">
      Loading files…
    </div>
  {:else if !listing.head}
    <div class="rounded-[8px] bg-white py-16 text-center text-sm text-brand-muted shadow-sm">
      Nothing pushed yet — the repo is empty until the first commit lands.
    </div>
  {:else}
    <div class="overflow-hidden rounded-[8px] bg-white shadow-sm">
      <div class="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-brand-border/40 px-4 py-3">
        <h3 class="font-heading text-[14px] font-semibold text-brand-text">Files</h3>
        <code
          class="rounded bg-brand-blue/5 px-1.5 py-0.5 font-mono text-[11px] text-brand-blue"
          title={listing.head.sha}
          data-testid="files-head-sha">{listing.head.sha.slice(0, 7)}</code
        >
        <span class="min-w-0 flex-1 truncate text-[12px] text-brand-text" title={listing.head.subject}>
          {listing.head.subject}
        </span>
        <span class="text-[11px] text-brand-muted">
          {listing.files.length}{listing.truncated ? '+' : ''} files ·
          {new Date(listing.head.author_time).toLocaleString()}
        </span>
        <button
          type="button"
          onclick={() => void loadTree()}
          disabled={treeLoading}
          class="rounded px-2 py-0.5 text-[11px] font-semibold text-brand-blue transition-colors hover:bg-brand-blue/10 disabled:opacity-40"
        >{treeLoading ? 'Loading…' : 'Refresh'}</button>
      </div>

      <div class="flex flex-col md:flex-row">
        <nav
          aria-label="Repository files"
          class="max-h-60 shrink-0 overflow-y-auto border-b border-brand-border/40 py-1 md:max-h-[70vh] md:w-64 md:border-r md:border-b-0"
        >
          {@render nodes(tree, 0)}
          {#if listing.truncated}
            <p class="px-3 py-2 text-[11px] text-brand-muted">
              Showing the first {listing.files.length} files.
            </p>
          {/if}
        </nav>

        <section class="min-w-0 flex-1" aria-label="File content">
          {#if selected}
            <div class="flex items-center justify-between gap-3 border-b border-brand-border/40 px-4 py-2">
              <span class="truncate font-mono text-[12px] text-brand-text" data-testid="files-selected-path">{selected}</span>
              {#if file}
                <span class="shrink-0 text-[11px] text-brand-muted">{formatBytes(file.size)}</span>
              {/if}
            </div>
          {/if}

          {#if fileError}
            <p class="px-4 py-10 text-center text-sm text-red-600">{fileError}</p>
          {:else if !selected}
            <p class="px-4 py-10 text-center text-sm text-brand-muted">Pick a file.</p>
          {:else if fileLoading || !file}
            <p class="px-4 py-10 text-center text-sm text-brand-muted">Loading…</p>
          {:else if file.too_large}
            <p class="px-4 py-10 text-center text-sm text-brand-muted">
              {formatBytes(file.size)} — too large to show here.
            </p>
          {:else if file.binary && IMAGE_EXT.test(file.path)}
            <div class="flex justify-center p-4">
              <img
                src="/api/sessions/{encodeURIComponent(sessionCode)}/players/{encodeURIComponent(playerId)}/repo-file?path={encodeURIComponent(file.path)}"
                alt={file.path}
                class="max-h-[60vh] max-w-full"
              />
            </div>
          {:else if file.binary}
            <p class="px-4 py-10 text-center text-sm text-brand-muted">
              Binary file ({formatBytes(file.size)}) — not shown.
            </p>
          {:else if file.content === ''}
            <p class="px-4 py-10 text-center text-sm text-brand-muted">Empty file.</p>
          {:else}
            <div class="flex max-h-[70vh] overflow-auto font-mono text-[12px] leading-5" data-testid="files-content">
              <pre
                aria-hidden="true"
                class="sticky left-0 shrink-0 select-none border-r border-brand-border/40 bg-white px-3 py-2 text-right text-brand-muted/70">{lineNumbers}</pre>
              <!-- One line on purpose: whitespace inside <pre> is content. shiki escapes the source, so its HTML is safe. -->
              <pre class="flex-1 px-3 py-2 text-brand-text" data-highlighted={highlightedHtml ? 'true' : undefined}>{#if highlightedHtml}{@html highlightedHtml}{:else}{file.content}{/if}</pre>
            </div>
          {/if}
        </section>
      </div>
    </div>
  {/if}
</div>
