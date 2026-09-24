<script lang="ts">
  // The repository a project's sessions start from, as two plain form
  // fields — `repo_url` and `repo_ref` — for the project editors. ololo
  // clones it into the player's folder before a session starts there.
  import { untrack } from "svelte";

  let {
    url = "",
    gitRef = "",
    idPrefix = "project",
  }: {
    url?: string | null;
    gitRef?: string | null;
    idPrefix?: string;
  } = $props();

  // Seeded from the props once; the fields are the user's from then on.
  let repoUrl = $state(untrack(() => url ?? ""));
  let repoRef = $state(untrack(() => gitRef ?? ""));
</script>

<div class="mb-4" data-testid="project-repo-fields">
  <div class="grid grid-cols-1 gap-3 md:grid-cols-[minmax(0,1fr)_220px]">
    <div class="min-w-0">
      <label for="{idPrefix}-repo-url" class="mb-1 block text-xs font-semibold text-brand-text">
        Repository <span class="font-normal text-brand-muted">(optional)</span>
      </label>
      <input
        id="{idPrefix}-repo-url"
        name="repo_url"
        type="text"
        inputmode="url"
        autocomplete="off"
        spellcheck="false"
        placeholder="https://github.com/org/starter.git"
        bind:value={repoUrl}
        data-testid="project-repo-url"
        class="h-[48px] w-full rounded-[8px] border-2 border-brand-border bg-white px-4 font-mono text-sm
               text-brand-text placeholder:font-body placeholder:text-brand-muted
               focus:border-brand-blue focus:outline-none"
      />
    </div>
    <div class="min-w-0">
      <label for="{idPrefix}-repo-ref" class="mb-1 block text-xs font-semibold text-brand-text">
        Branch, tag or commit
      </label>
      <input
        id="{idPrefix}-repo-ref"
        name="repo_ref"
        type="text"
        autocomplete="off"
        spellcheck="false"
        placeholder="default branch"
        bind:value={repoRef}
        data-testid="project-repo-ref"
        class="h-[48px] w-full rounded-[8px] border-2 border-brand-border bg-white px-4 font-mono text-sm
               text-brand-text placeholder:font-body placeholder:text-brand-muted
               focus:border-brand-blue focus:outline-none"
      />
    </div>
  </div>
  <p class="mt-1 text-xs text-brand-muted">
    The code every session starts from: ololo clones it into the player's folder before the
    session starts there. An https:// or ssh:// URL, or git@host:org/repo — no credentials in it.
  </p>
</div>
