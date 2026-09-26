<script lang="ts">
  import { onMount } from "svelte";
  import type { PageData, ActionData } from "./$types";
  import PersonalProjectForm from "$lib/components/projects/PersonalProjectForm.svelte";
  import { readProjectDraft, type ProjectDraft } from "$lib/project-draft";

  let { data, form }: { data: PageData; form: ActionData } = $props();

  // The project the landing drafted ("Edit project"), handed over in this
  // tab's storage — the server cannot see it, so the form starts empty and
  // is rebuilt around it once the page is in the browser.
  let draft = $state<ProjectDraft | null>(null);
  onMount(() => {
    if (data.draftRequested && !form) draft = readProjectDraft();
  });
  const fromDraft = $derived(
    draft && {
      ...draft,
      session_duration_secs: draft.session_duration_secs || data.options.session.default_secs,
    },
  );
</script>

<svelte:head>
  <title>New project — ololo.dev</title>
</svelte:head>

<div class="-mx-6 -mt-8 min-h-screen bg-brand-light-blue">
  <div class="mx-auto w-full max-w-[1206px] px-[18px] py-10 md:py-[64px]">
    <h1 class="font-heading text-[34px] font-bold leading-[1.18] text-brand-text">
      New project
    </h1>
    <p class="mb-8 mt-3 max-w-[680px] text-[15px] leading-relaxed text-brand-muted">
      Bring your own work. Describe it, split it into tasks if you like, and play it as a
      session in your own repository: judges review every task and code health is tracked at
      each step.
    </p>
    {#if data.isAdmin}
      <p class="-mt-4 mb-8 text-sm">
        <a href="/projects/new/challenge" class="font-semibold text-brand-blue hover:opacity-70"
          >Create a challenge project for the catalog instead →</a
        >
      </p>
    {/if}

    {#key draft}
      <PersonalProjectForm
        options={data.options}
        initial={form?.values ?? fromDraft ?? data.initial}
        submitLabel="Create project"
        cancelHref="/projects"
        error={form ?? null}
      />
    {/key}
  </div>
</div>
