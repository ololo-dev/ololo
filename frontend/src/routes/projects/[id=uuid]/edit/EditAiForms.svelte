<script lang="ts">
  import { enhance } from '$app/forms';
  import { invalidateAll } from '$app/navigation';
  import type { TaskDraft } from '$lib/api';

  interface Props {
    draftDescription?: string;
    draftTasks?: TaskDraft[] | null;
    importResult?: { imported: number; failed: number; total: number } | null;
    beautifyLoading?: boolean;
    beautifyError?: string | null;
    generateLoading?: boolean;
    generateError?: string | null;
    onGenerated?: () => void;
  }

  let {
    draftDescription = $bindable(''),
    draftTasks = $bindable(null),
    importResult = $bindable(null),
    beautifyLoading = $bindable(false),
    beautifyError = $bindable(null),
    generateLoading = $bindable(false),
    generateError = $bindable(null),
    onGenerated,
  }: Props = $props();
</script>

<!-- Beautify Description -->
<form
  id="beautify-form"
  method="POST"
  action="?/beautifyDescription"
  class="hidden"
  use:enhance={() => {
    beautifyLoading = true;
    beautifyError = null;
    return async ({ result }) => {
      beautifyLoading = false;
      if (result.type === 'success' && result.data?.description) {
        draftDescription = result.data.description as string;
      } else if (result.type === 'failure') {
        beautifyError = (result.data?.error as string) ?? 'error';
      }
    };
  }}
>
  <input type="hidden" name="description" bind:value={draftDescription} />
</form>

<!-- Generate Tasks -->
<form
  id="generate-tasks-form"
  method="POST"
  action="?/generateTasks"
  class="hidden"
  use:enhance={() => {
    generateLoading = true;
    generateError = null;
    draftTasks = null;
    return async ({ result }) => {
      generateLoading = false;
      if (result.type === 'success' && result.data?.tasks) {
        draftTasks = result.data.tasks as TaskDraft[];
        onGenerated?.();
      } else if (result.type === 'failure') {
        generateError = (result.data?.error as string) ?? 'error';
      }
    };
  }}
>
  <input type="hidden" name="description" bind:value={draftDescription} />
</form>

<!-- Import Tasks -->
<form
  id="import-tasks-form"
  method="POST"
  action="?/importTasks"
  class="hidden"
  use:enhance={() => {
    return async ({ result }) => {
      if (result.type === 'success') {
        importResult = {
          imported: result.data?.imported as number,
          failed: result.data?.failed as number,
          total: result.data?.total as number,
        };
        await invalidateAll();
      }
    };
  }}
>
  <input type="hidden" name="tasks_json" value={JSON.stringify(draftTasks ?? [])} />
</form>
