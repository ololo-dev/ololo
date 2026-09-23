<script lang="ts">
  import type { PageData, ActionData } from "./$types";
  import PersonalProjectForm from "$lib/components/projects/PersonalProjectForm.svelte";

  let { data, form }: { data: PageData; form: ActionData } = $props();
  const project = $derived(data.detail.project);
</script>

<svelte:head>
  <title>Edit {project.name} — ololo.dev</title>
</svelte:head>

<div class="-mx-6 -mt-8 min-h-screen bg-brand-light-blue">
  <div class="mx-auto w-full max-w-[900px] px-[18px] py-10 md:py-[64px]">
    <a href="/projects/{project.id}" class="text-sm font-semibold text-brand-blue hover:opacity-70"
      >← {project.name}</a
    >
    <h1 class="mt-3 font-heading text-[34px] font-bold leading-[1.18] text-brand-text">
      Edit project
    </h1>

    {#if data.detail.editable}
      <p class="mb-8 mt-3 max-w-[680px] text-[15px] leading-relaxed text-brand-muted">
        Saving rebuilds every task from what you write here. The start command stays the same.
      </p>
      <PersonalProjectForm
        options={data.options}
        initial={form?.values ?? data.detail.spec}
        submitLabel="Save changes"
        cancelHref="/projects/{project.id}"
        error={form ?? null}
      />
    {:else}
      <div class="mt-6 rounded-[8px] bg-white px-6 py-6 md:px-[48px]" data-testid="pp-frozen">
        <p class="text-[15px] leading-relaxed text-brand-text">
          This project has been played {data.detail.session_count}
          {data.detail.session_count === 1 ? "time" : "times"}, so its tasks stay as they were —
          the sessions' results refer to them. Duplicate it to change the tasks and play the
          new version.
        </p>
        <a
          href="/projects/new?from={project.id}"
          data-testid="pp-duplicate"
          class="mt-5 inline-block rounded-btn bg-brand-blue px-6 py-3 text-sm font-semibold text-white hover:opacity-80"
        >
          Duplicate and edit
        </a>
      </div>
    {/if}
  </div>
</div>
