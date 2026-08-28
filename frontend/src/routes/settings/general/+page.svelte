<script lang="ts">
  import { enhance } from '$app/forms';
  import { untrack } from 'svelte';
  import { notify } from '$lib/notifications.svelte';

  let { data } = $props();

  let allowProjectCreation = $state(untrack(() => data.allowUserProjectCreation));
  $effect(() => { allowProjectCreation = data.allowUserProjectCreation; });

  let sessionReplay = $state(untrack(() => data.sessionReplayEnabled));
  $effect(() => { sessionReplay = data.sessionReplayEnabled; });

</script>

<div class="mt-8 flex flex-col gap-8">

  <section>
    <div class="mb-4">
      <h2 class="font-heading text-[20px] font-semibold text-brand-text">Platform</h2>
      <p class="mt-0.5 text-sm text-brand-muted">Control which users can perform platform-level actions.</p>
    </div>
    <div class="rounded-[8px] bg-white shadow-sm">
      <form
        method="POST"
        action="?/updateProjectCreation"
        use:enhance={() => {
          let savingProjectCreation = true;
          return async ({ result, update }) => {
            await update({ reset: false });
            savingProjectCreation = false;
            if (result.type === 'success' && result.data?.success) {
              notify.success('Platform setting updated.', 'Settings');
            } else if (result.type === 'success') {
              notify.error('An error occurred. Please try again.', 'Settings error');
            }
          };
        }}
      >
        <div class="px-[100px] py-6 max-md:px-6">
          <div class="mb-1 text-xs font-semibold text-brand-text">User Project Creation</div>
          <label class="flex cursor-pointer items-center gap-3">
            <div class="relative">
              <input
                type="checkbox"
                class="sr-only"
                checked={allowProjectCreation}
                oninput={(e) => (allowProjectCreation = (e.target as HTMLInputElement).checked)}
              />
              <div
                class="h-6 w-11 rounded-full transition-colors duration-200"
                class:bg-brand-blue={allowProjectCreation}
                class:bg-gray-300={!allowProjectCreation}
              ></div>
              <div
                class="absolute left-1 top-1 h-4 w-4 rounded-full bg-white shadow transition-transform duration-200"
                style:transform={allowProjectCreation ? 'translateX(20px)' : 'translateX(0)'}
              ></div>
            </div>
            <span class="text-sm text-brand-text">
              Allow non-admin users to create projects
            </span>
          </label>
          <p class="mt-1 text-xs text-brand-muted">
            When disabled, only administrators can create new projects.
          </p>
          <input type="hidden" name="allow_creation" value={allowProjectCreation ? 'true' : 'false'} />
        </div>
        <div class="flex justify-end border-t border-brand-border px-[100px] py-4 max-md:px-6">
          <button
            type="submit"
            class="rounded-btn bg-brand-blue px-6 py-2 text-sm font-semibold text-white
                   transition-opacity hover:opacity-80"
          >
            Save
          </button>
        </div>
      </form>
    </div>
  </section>

  <section>
    <div class="mb-4">
      <h2 class="font-heading text-[20px] font-semibold text-brand-text">Sessions</h2>
      <p class="mt-0.5 text-sm text-brand-muted">What a finished session offers on its pages.</p>
    </div>
    <div class="rounded-[8px] bg-white shadow-sm">
      <form
        method="POST"
        action="?/updateSessionReplay"
        use:enhance={() => {
          return async ({ result, update }) => {
            await update({ reset: false });
            if (result.type === 'success' && result.data?.success) {
              notify.success('Session setting updated.', 'Settings');
            } else if (result.type === 'success') {
              notify.error('An error occurred. Please try again.', 'Settings error');
            }
          };
        }}
      >
        <div class="px-[100px] py-6 max-md:px-6">
          <div class="mb-1 text-xs font-semibold text-brand-text">Replay Panel</div>
          <label class="flex cursor-pointer items-center gap-3">
            <div class="relative">
              <input
                type="checkbox"
                class="sr-only"
                data-testid="replay-setting-toggle"
                checked={sessionReplay}
                oninput={(e) => (sessionReplay = (e.target as HTMLInputElement).checked)}
              />
              <div
                class="h-6 w-11 rounded-full transition-colors duration-200"
                class:bg-brand-blue={sessionReplay}
                class:bg-gray-300={!sessionReplay}
              ></div>
              <div
                class="absolute left-1 top-1 h-4 w-4 rounded-full bg-white shadow transition-transform duration-200"
                style:transform={sessionReplay ? 'translateX(20px)' : 'translateX(0)'}
              ></div>
            </div>
            <span class="text-sm text-brand-text">
              Offer the replay bar on finished sessions
            </span>
          </label>
          <p class="mt-1 text-xs text-brand-muted">
            The replay sweeps a playhead through a finished session and re-reveals its
            events up to that moment. Administrators only, and only while this is on —
            when it is off, nobody sees the bar.
          </p>
          <input type="hidden" name="replay_enabled" value={sessionReplay ? 'true' : 'false'} />
        </div>
        <div class="flex justify-end border-t border-brand-border px-[100px] py-4 max-md:px-6">
          <button
            type="submit"
            class="rounded-btn bg-brand-blue px-6 py-2 text-sm font-semibold text-white
                   transition-opacity hover:opacity-80"
          >
            Save
          </button>
        </div>
      </form>
    </div>
  </section>

</div>
