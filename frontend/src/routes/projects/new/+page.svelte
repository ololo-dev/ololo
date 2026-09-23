<script lang="ts">
  import type { PageData, ActionData } from "./$types";
  import PersonalProjectForm from "$lib/components/projects/PersonalProjectForm.svelte";

  let { data, form }: { data: PageData; form: ActionData } = $props();
</script>

<svelte:head>
  <title>New project — ololo.dev</title>
</svelte:head>

<div class="-mx-6 -mt-8 min-h-screen bg-brand-light-blue">
  <div class="mx-auto w-full max-w-[900px] px-[18px] py-10 md:py-[64px]">
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

    <PersonalProjectForm
      options={data.options}
      initial={form?.values ?? data.initial}
      submitLabel="Create project"
      cancelHref="/projects"
      error={form ?? null}
    />
  </div>
</div>
