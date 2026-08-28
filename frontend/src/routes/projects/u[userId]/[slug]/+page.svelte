<script lang="ts">
  import type { PageData } from './$types';
  import MarkdownContent from '$lib/components/MarkdownContent.svelte';
  let { data }: { data: PageData } = $props();
</script>

<svelte:head>
  <title>{data.project.name} — ololo.dev</title>
</svelte:head>

<div class="flex items-center justify-between">
  <h1 class="font-heading text-2xl font-bold text-slate-900">{data.project.name}</h1>
  <div class="flex gap-2">
    <a
      href="/projects/{data.project.id}"
      class="rounded-lg border border-slate-200 px-3 py-1.5 text-sm text-slate-600 transition-colors hover:bg-slate-50"
    >
      Full view
    </a>
    <a href="/projects" class="rounded-lg border border-slate-200 px-3 py-1.5 text-sm text-slate-600 transition-colors hover:bg-slate-50">
      Back
    </a>
  </div>
</div>

<p class="mt-1 text-xs text-slate-500">
  {data.project.public ? 'Public' : 'Private'} · {data.tasks.length} task{data.tasks.length === 1 ? '' : 's'}
</p>

{#if data.project.description}
  <div class="prose mt-3 text-sm">
    <MarkdownContent value={data.project.description} />
  </div>
{/if}

<section class="mt-6">
  <h2 class="font-heading text-lg font-semibold text-slate-900">Tasks</h2>
  {#if data.tasks.length === 0}
    <p class="mt-4 text-sm text-slate-500">No tasks yet.</p>
  {:else}
    <div class="mt-4 overflow-hidden rounded-xl border border-slate-200 bg-white">
    <ol class="">
      {#each data.tasks as task (task.id)}
        <li class="border-b border-slate-100 px-4 py-3 last:border-0 transition-colors hover:bg-slate-50">
          <div class="text-sm font-medium">#{task.ordinal} — {task.title}</div>
          <div class="text-xs text-slate-500">{task.test_template.kind}</div>
        </li>
      {/each}
    </ol>
    </div>
  {/if}
</section>
