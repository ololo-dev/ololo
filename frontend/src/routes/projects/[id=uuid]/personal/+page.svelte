<script lang="ts">
  import type { PageData, ActionData } from "./$types";
  import { enhance } from "$app/forms";
  import PersonalProjectForm from "$lib/components/projects/PersonalProjectForm.svelte";

  let { data, form }: { data: PageData; form: ActionData } = $props();
  const project = $derived(data.detail.project);
</script>

<svelte:head>
  <title>Edit {project.name} — ololo.dev</title>
</svelte:head>

<div class="-mx-6 -mt-8 min-h-screen bg-brand-light-blue">
  <div class="mx-auto w-full max-w-[1206px] px-[18px] py-10 md:py-[64px]">
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
        initial={form?.values ?? { ...data.detail.spec, public: project.public }}
        submitLabel="Save changes"
        cancelHref="/projects/{project.id}"
        error={form?.error ? { error: form.error, field: form.field, detail: form.detail } : null}
      />
    {:else}
      <div class="max-w-[760px]">
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

      <!-- Who sees it is not part of the tasks, so it still moves. -->
      <form
        method="POST"
        action="?/visibility"
        use:enhance
        class="mt-6 rounded-[8px] bg-white px-6 py-6 md:px-[48px]"
        data-testid="pp-visibility-form"
      >
        <h2 class="font-heading text-[20px] font-bold text-brand-text">Who sees it</h2>
        <p class="mt-1 text-[15px] leading-relaxed text-brand-muted" data-testid="pp-visibility-now">
          {project.public
            ? "Public: listed on your profile, and anyone can open it and watch its sessions."
            : "Private: only you see it; players join a session by its code."}
        </p>
        {#if project.public && !data.options.private_allowed}
          <a
            href="/pricing"
            data-testid="pp-private-upsell"
            class="mt-4 inline-block text-sm font-semibold text-brand-blue hover:opacity-70"
            >Private projects come with Premium →</a
          >
        {:else}
          <input type="hidden" name="public" value={project.public ? "false" : "true"} />
          <button
            type="submit"
            data-testid="pp-visibility-toggle"
            class="mt-4 rounded-btn border border-brand-blue px-5 py-2.5 text-sm font-semibold text-brand-blue transition-colors hover:bg-brand-light-blue"
          >
            {project.public ? "Make it private" : "Make it public"}
          </button>
        {/if}
        {#if form?.visibilityError}
          <p class="mt-3 text-sm text-red-500" data-testid="pp-visibility-error">
            {form.visibilityError === "premium_required"
              ? "Private projects are part of Premium."
              : "Could not change who sees it. Please try again."}
          </p>
        {/if}
      </form>
      </div>
    {/if}
  </div>
</div>
