<script lang="ts">
  import { enhance } from '$app/forms';
  import type { PageData, ActionData } from './$types';
  import MarkdownField from '$lib/components/MarkdownField.svelte';
  import TagInput from '$lib/components/TagInput.svelte';
  import CoverImageUpload from '$lib/components/CoverImageUpload.svelte';
  import ProjectSessionDurationField from '$lib/components/ProjectSessionDurationField.svelte';
  import { untrack } from 'svelte';

  let { data, form }: { data: PageData; form: ActionData } = $props();

  let nameValue = $state(untrack(() => form?.name ?? ''));
  let descriptionValue = $state(untrack(() => form?.description ?? ''));
  $effect(() => {
    nameValue = form?.name ?? '';
    descriptionValue = form?.description ?? '';
  });

  let categoryValue = $state('');
  let tags = $state<string[]>([]);
  let coverImageUrl = $state<string | null>(null);

  // Points defaults (admin only) — empty string means "use baseline".
  let pointsValue = $state('');
  let pointsFail = $state('');
  let pointsNoResponse = $state('');
  let pointsCompletionBonus = $state('');
  let pointsOpen = $state(false);

  // Intervals defaults (admin only) — empty string means "use baseline".
  let intervalsDeadline = $state('');
  let intervalsMin = $state('');
  let intervalsIncrement = $state('');
  let intervalsMax = $state('');
  let intervalsOpen = $state(false);

  // Session duration default (admin only) — empty string means "use default".
  let sessionDuration = $state('');

  let detailsOpen = $state(true);
  let publishOpen = $state(true);
</script>

<svelte:head>
  <title>New project — ololo.dev</title>
</svelte:head>

<div class="-mx-6 -mt-8 min-h-screen bg-brand-light-blue">
  <div class="mx-auto w-full max-w-[1206px] px-[18px] py-10 md:py-[88px]">

    <!-- Page title -->
    <h1 class="font-heading text-[34px] font-bold leading-[1.18] text-brand-text">
      New project
    </h1>

    <form method="POST" use:enhance>

      <!-- Action bar -->
      <div class="mt-8 flex items-center justify-between border-b-2 border-brand-border pb-3">
        <span></span>
        <div class="flex items-center gap-3">
          <a
            href="/projects"
            class="text-sm font-semibold text-brand-blue transition-opacity hover:opacity-70"
          >
            Cancel
          </a>
          <button
            type="submit"
            data-testid="create-project"
            class="rounded-btn bg-brand-blue px-6 py-2 text-sm font-semibold text-white
                   transition-opacity hover:opacity-80"
          >
            Create project
          </button>
        </div>
      </div>

      <!-- Details accordion -->
      <div class="mt-[48px]">
        <button
          type="button"
          class="mb-8 flex w-full items-center justify-between"
          onclick={() => (detailsOpen = !detailsOpen)}
        >
          <span class="font-heading text-[26px] font-bold leading-[1.23] text-brand-text">
            Details
          </span>
          <svg
            width="24" height="24" viewBox="0 0 24 24" fill="none"
            xmlns="http://www.w3.org/2000/svg"
            class="text-brand-muted transition-transform duration-200"
            style:transform={detailsOpen ? 'rotate(180deg)' : 'rotate(90deg)'}
            aria-hidden="true"
          >
            <path d="M6 9l6 6 6-6" stroke="currentColor" stroke-width="2"
              stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>

        {#if detailsOpen}
          <div class="rounded-[8px] bg-white">
            <div class="px-[100px] py-6 max-md:px-6">

              <!-- Project name -->
              <div class="mb-4">
                <label for="name" class="mb-1 block text-xs font-semibold text-brand-text">
                  Project name
                </label>
                <input
                  id="name"
                  name="name"
                  type="text"
                  required
                  minlength="1"
                  maxlength="200"
                  placeholder="e.g. My coding challenge"
                  value={nameValue}
                  oninput={(e) => (nameValue = (e.target as HTMLInputElement).value)}
                  class="h-[48px] w-full rounded-[8px] border-2 border-brand-border bg-white px-4
                         text-base text-brand-text placeholder:text-brand-muted
                         focus:border-brand-blue focus:outline-none"
                  data-testid="project-name"
                />
              </div>

              <!-- Description -->
              <div>
                <div class="mb-1 text-xs font-semibold text-brand-text">
                  Description
                  <span class="font-normal text-brand-muted">(optional)</span>
                </div>
                <MarkdownField
                  name="description"
                  bind:value={descriptionValue}
                  placeholder="Optional project description"
                  data-testid="project-description"
                />
              </div>

              <!-- Category -->
              <div class="mb-4 mt-4">
                <label for="category" class="mb-1 block text-xs font-semibold text-brand-text">
                  Category <span class="font-normal text-brand-muted">(optional)</span>
                </label>
                <select
                  id="category"
                  name="category"
                  value={categoryValue}
                  onchange={(e) => (categoryValue = (e.target as HTMLSelectElement).value)}
                  class="h-[48px] w-full rounded-[8px] border-2 border-brand-border bg-white px-4
                         text-base text-brand-text focus:border-brand-blue focus:outline-none"
                >
                  <option value="">No category</option>
                  {#each data.categories as cat}
                    <option value={cat}>{cat}</option>
                  {/each}
                </select>
              </div>

              <!-- Tags -->
              <div class="mb-4">
                <div class="mb-1 text-xs font-semibold text-brand-text">
                  Tags <span class="font-normal text-brand-muted">(optional, max 10)</span>
                </div>
                <TagInput {tags} onchange={(t) => (tags = t)} />
                <input type="hidden" name="tags_json" value={JSON.stringify(tags)} />
              </div>

              <!-- Cover image -->
              <div class="mb-4">
                <div class="mb-1 text-xs font-semibold text-brand-text">
                  Cover image <span class="font-normal text-brand-muted">(optional)</span>
                </div>
                <CoverImageUpload value={coverImageUrl} onchange={(url) => (coverImageUrl = url)} />
                <input type="hidden" name="cover_image_url" value={coverImageUrl ?? ''} />
              </div>

              {#if data.isAdmin}
                <!-- Points defaults (admin only, collapsed by default) -->
                <div class="mb-4 mt-6 border-t border-brand-border pt-4">
                  <button
                    type="button"
                    class="flex w-full items-center justify-between"
                    onclick={() => (pointsOpen = !pointsOpen)}
                  >
                    <span class="text-xs font-semibold text-brand-text">
                      Points defaults <span class="font-normal text-brand-muted">(optional — baseline: 10 / -5 / -10 / 10)</span>
                    </span>
                    <svg
                      width="16" height="16" viewBox="0 0 24 24" fill="none"
                      xmlns="http://www.w3.org/2000/svg"
                      class="text-brand-muted transition-transform duration-200"
                      style:transform={pointsOpen ? 'rotate(180deg)' : 'rotate(90deg)'}
                      aria-hidden="true"
                    >
                      <path d="M6 9l6 6 6-6" stroke="currentColor" stroke-width="2"
                        stroke-linecap="round" stroke-linejoin="round" />
                    </svg>
                  </button>

                  {#if pointsOpen}
                    <div class="mt-3 grid grid-cols-1 gap-3 sm:grid-cols-2">
                      <div>
                        <label for="points_value" class="mb-1 block text-xs font-semibold text-brand-text">
                          Point value <span class="font-normal text-brand-muted">(display + pass score)</span>
                        </label>
                        <input
                          id="points_value"
                          name="points_value"
                          type="number"
                          placeholder="10"
                          value={pointsValue}
                          oninput={(e) => (pointsValue = (e.target as HTMLInputElement).value)}
                          class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3
                                 text-base text-brand-text placeholder:text-brand-muted
                                 focus:border-brand-blue focus:outline-none"
                        />
                      </div>
                      <div>
                        <label for="points_fail" class="mb-1 block text-xs font-semibold text-brand-text">
                          Fail points
                        </label>
                        <input
                          id="points_fail"
                          name="points_fail"
                          type="number"
                          placeholder="-5"
                          value={pointsFail}
                          oninput={(e) => (pointsFail = (e.target as HTMLInputElement).value)}
                          class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3
                                 text-base text-brand-text placeholder:text-brand-muted
                                 focus:border-brand-blue focus:outline-none"
                        />
                      </div>
                      <div>
                        <label for="points_no_response" class="mb-1 block text-xs font-semibold text-brand-text">
                          No-response points
                        </label>
                        <input
                          id="points_no_response"
                          name="points_no_response"
                          type="number"
                          placeholder="-10"
                          value={pointsNoResponse}
                          oninput={(e) => (pointsNoResponse = (e.target as HTMLInputElement).value)}
                          class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3
                                 text-base text-brand-text placeholder:text-brand-muted
                                 focus:border-brand-blue focus:outline-none"
                        />
                      </div>
                      <div>
                        <label for="points_completion_bonus" class="mb-1 block text-xs font-semibold text-brand-text">
                          Completion bonus
                        </label>
                        <input
                          id="points_completion_bonus"
                          name="points_completion_bonus"
                          type="number"
                          placeholder="10"
                          value={pointsCompletionBonus}
                          oninput={(e) => (pointsCompletionBonus = (e.target as HTMLInputElement).value)}
                          class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3
                                 text-base text-brand-text placeholder:text-brand-muted
                                 focus:border-brand-blue focus:outline-none"
                        />
                      </div>
                    </div>
                  {/if}
                </div>
              {/if}

              {#if data.isAdmin}
                <!-- Intervals defaults (admin only, collapsed by default) -->
                <div class="mb-4 mt-6 border-t border-brand-border pt-4">
                  <button
                    type="button"
                    class="flex w-full items-center justify-between"
                    onclick={() => (intervalsOpen = !intervalsOpen)}
                  >
                    <span class="text-xs font-semibold text-brand-text">
                      Intervals defaults <span class="font-normal text-brand-muted">(optional — baseline: 60 / 5 / 5 / 60)</span>
                    </span>
                    <svg
                      width="16" height="16" viewBox="0 0 24 24" fill="none"
                      xmlns="http://www.w3.org/2000/svg"
                      class="text-brand-muted transition-transform duration-200"
                      style:transform={intervalsOpen ? 'rotate(180deg)' : 'rotate(90deg)'}
                      aria-hidden="true"
                    >
                      <path d="M6 9l6 6 6-6" stroke="currentColor" stroke-width="2"
                        stroke-linecap="round" stroke-linejoin="round" />
                    </svg>
                  </button>

                  {#if intervalsOpen}
                    <div class="mt-3 grid grid-cols-1 gap-3 sm:grid-cols-2">
                      <div>
                        <label for="intervals_deadline_secs" class="mb-1 block text-xs font-semibold text-brand-text">
                          Deadline (secs)
                        </label>
                        <input
                          id="intervals_deadline_secs"
                          name="intervals_deadline_secs"
                          type="number"
                          placeholder="60"
                          value={intervalsDeadline}
                          oninput={(e) => (intervalsDeadline = (e.target as HTMLInputElement).value)}
                          class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3
                                 text-base text-brand-text placeholder:text-brand-muted
                                 focus:border-brand-blue focus:outline-none"
                        />
                      </div>
                      <div>
                        <label for="intervals_min_interval_secs" class="mb-1 block text-xs font-semibold text-brand-text">
                          Min interval (secs)
                        </label>
                        <input
                          id="intervals_min_interval_secs"
                          name="intervals_min_interval_secs"
                          type="number"
                          placeholder="5"
                          value={intervalsMin}
                          oninput={(e) => (intervalsMin = (e.target as HTMLInputElement).value)}
                          class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3
                                 text-base text-brand-text placeholder:text-brand-muted
                                 focus:border-brand-blue focus:outline-none"
                        />
                      </div>
                      <div>
                        <label for="intervals_interval_increment_secs" class="mb-1 block text-xs font-semibold text-brand-text">
                          Increment (secs)
                        </label>
                        <input
                          id="intervals_interval_increment_secs"
                          name="intervals_interval_increment_secs"
                          type="number"
                          placeholder="5"
                          value={intervalsIncrement}
                          oninput={(e) => (intervalsIncrement = (e.target as HTMLInputElement).value)}
                          class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3
                                 text-base text-brand-text placeholder:text-brand-muted
                                 focus:border-brand-blue focus:outline-none"
                        />
                      </div>
                      <div>
                        <label for="intervals_max_interval_secs" class="mb-1 block text-xs font-semibold text-brand-text">
                          Max interval (secs)
                        </label>
                        <input
                          id="intervals_max_interval_secs"
                          name="intervals_max_interval_secs"
                          type="number"
                          placeholder="60"
                          value={intervalsMax}
                          oninput={(e) => (intervalsMax = (e.target as HTMLInputElement).value)}
                          class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3
                                 text-base text-brand-text placeholder:text-brand-muted
                                 focus:border-brand-blue focus:outline-none"
                        />
                      </div>
                    </div>
                  {/if}
                </div>
              {/if}

              {#if data.isAdmin}
                <!-- Session duration default (admin only) -->
                <ProjectSessionDurationField
                  value={sessionDuration}
                  onchange={(v) => (sessionDuration = v)}
                />
              {/if}

              {#if form?.error}
                <p class="mt-4 text-sm text-red-500" data-testid="create-error">
                  {#if form.error === 'invalid_name'}
                    Project name is required.
                  {:else if form.error === 'invalid_session_duration'}
                    Session duration must be between 60 and 86400 seconds.
                  {:else}
                    An error occurred. Please try again.
                  {/if}
                </p>
              {/if}

            </div>
          </div>
        {/if}
      </div>

      <!-- Publish accordion — admin only; non-admins always create private projects -->
      {#if data.isAdmin}
      <div class="mt-[48px]">
        <button
          type="button"
          class="mb-8 flex w-full items-center justify-between"
          onclick={() => (publishOpen = !publishOpen)}
        >
          <span class="font-heading text-[26px] font-bold leading-[1.23] text-brand-text">
            Publish
          </span>
          <svg
            width="24" height="24" viewBox="0 0 24 24" fill="none"
            xmlns="http://www.w3.org/2000/svg"
            class="text-brand-muted transition-transform duration-200"
            style:transform={publishOpen ? 'rotate(180deg)' : 'rotate(90deg)'}
            aria-hidden="true"
          >
            <path d="M6 9l6 6 6-6" stroke="currentColor" stroke-width="2"
              stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>

        {#if publishOpen}
          <div class="rounded-[8px] bg-white">
            <div class="px-[100px] py-6 max-md:px-6">

              <!-- Public project checkbox — defaults to checked -->
              <div class="mb-4 flex items-start gap-3">
                <input
                  id="public"
                  name="public"
                  type="checkbox"
                  value="true"
                  checked={!form || form.public === 'true'}
                  class="mt-0.5 rounded border-brand-border"
                  data-testid="project-public"
                />
                <div>
                  <label for="public" class="text-sm font-semibold text-brand-text">
                    Public project
                  </label>
                  <p class="mt-0.5 text-xs text-brand-muted">Share your project publicly</p>
                </div>
              </div>

            </div>

            <!-- Card footer: create button flush right -->
            <div class="flex justify-end border-t border-brand-border px-[100px] py-4 max-md:px-6">
              <button
                type="submit"
                class="rounded-btn bg-brand-blue px-6 py-2 text-sm font-semibold text-white
                       transition-opacity hover:opacity-80"
              >
                Create project
              </button>
            </div>
          </div>
        {/if}
      </div>
      {/if}

    </form>
  </div>
</div>
