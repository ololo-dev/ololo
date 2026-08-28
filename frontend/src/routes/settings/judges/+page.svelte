<script lang="ts">
  import { invalidateAll } from '$app/navigation';
  import { untrack } from 'svelte';
  import { notify } from '$lib/notifications.svelte';
  import {
    createJudge,
    updateJudge,
    deleteJudge,
    syncJudges,
    ApiError,
    type Judge,
    type LlmProvider,
    type LlmPool,
    type LlmSourceOrder,
    type RatingScale,
    type JudgeUsage,
  } from '$lib/api';
  import { modelSuggestions } from '$lib/llm-models';
  import ModelSelector from '$lib/components/ModelSelector.svelte';
  import CoverImageUpload from '$lib/components/CoverImageUpload.svelte';
  import { ikAvatar } from '$lib/imagekit';

  let { data } = $props();

  let judges = $state<Judge[]>(untrack(() => [...data.judges]));
  $effect(() => { judges = [...data.judges]; });

  // Configured LLM providers and pools for the optional per-judge override.
  const providers = $derived<LlmProvider[]>(data.providers ?? []);
  const pools = $derived<LlmPool[]>(data.pools ?? []);

  /** Suggestion loader for `ModelSelector`; cached per provider in $lib/llm-models. */
  function suggestionsFor(providerId: string): Promise<string[]> {
    const p = providers.find((x) => x.id === providerId);
    return p ? modelSuggestions(p) : Promise.resolve([]);
  }

  type EditorMode = 'create' | 'edit' | null;
  let editorMode = $state<EditorMode>(null);

  const emptyForm = (): {
    slug: string;
    name: string;
    description: string;
    prompt: string;
    min: string;
    max: string;
    step: string;
    llmProviderId: string;
    llmModel: string;
    llmPoolId: string;
    llmSourceOrder: LlmSourceOrder;
    avatarUrl: string | null;
  } => ({
    slug: '',
    name: '',
    description: '',
    prompt: '',
    min: '0',
    max: '10',
    step: '0.5',
    llmProviderId: '',
    llmModel: '',
    llmPoolId: '',
    llmSourceOrder: 'pool_first',
    avatarUrl: null,
  });

  let form = $state(untrack(emptyForm));
  let editingId = $state<string | null>(null);
  let saving = $state(false);
  let formError = $state<string | undefined>(undefined);
  let deletingId = $state<string | null>(null);

  function scaleSummary(rs: RatingScale): string {
    return `${rs.min}–${rs.max} step ${rs.step}`;
  }

  // ── Where each judge runs, and what it has done ────────────────────────
  const usageById = $derived(
    new Map<string, JudgeUsage>((data.usage ?? []).map((u: JudgeUsage) => [u.judge_id, u])),
  );

  /** Attachments folded to one entry per project, tasks kept in order. */
  function projectsOf(usage: JudgeUsage | undefined) {
    const byProject = new Map<
      string,
      { id: string; name: string; slug: string | null; tasks: { ordinal: number; title: string }[] }
    >();
    for (const a of usage?.attachments ?? []) {
      const entry = byProject.get(a.project_id) ?? {
        id: a.project_id,
        name: a.project_name,
        slug: a.project_slug,
        tasks: [],
      };
      entry.tasks.push({ ordinal: a.task_ordinal, title: a.task_title });
      byProject.set(a.project_id, entry);
    }
    return [...byProject.values()];
  }

  /** `1 234` — thousands are common once a judge has been around. */
  const count = (n: number) => n.toLocaleString('en-US');
  /** `+40` / `−15`, with the sign a person reads rather than a hyphen. */
  const signed = (n: number) => (n > 0 ? `+${count(n)}` : n < 0 ? `−${count(-n)}` : '0');

  function lastRun(iso: string | null): string {
    if (!iso) return 'never run';
    const days = Math.floor((Date.now() - Date.parse(iso)) / 86_400_000);
    if (days <= 0) return 'today';
    if (days === 1) return 'yesterday';
    if (days < 30) return `${days} days ago`;
    return new Date(iso).toLocaleDateString();
  }

  /** Judges whose attachment list is open. */
  let expanded = $state(new Set<string>());
  function toggle(id: string) {
    const next = new Set(expanded);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    expanded = next;
  }

  function startCreate() {
    editorMode = 'create';
    editingId = null;
    form = emptyForm();
    formError = undefined;
  }

  function startEdit(j: Judge) {
    editorMode = 'edit';
    editingId = j.id;
    form = {
      slug: j.slug,
      name: j.name,
      description: j.description,
      prompt: j.prompt,
      min: String(j.rating_scale.min),
      max: String(j.rating_scale.max),
      step: String(j.rating_scale.step),
      llmProviderId: j.llm_provider_id ?? '',
      llmModel: j.llm_model ?? '',
      llmPoolId: j.llm_pool_id ?? '',
      llmSourceOrder: j.llm_source_order,
      avatarUrl: j.avatar_url ?? null,
    };
    formError = undefined;
  }

  function cancelEditor() {
    editorMode = null;
    editingId = null;
    formError = undefined;
  }

  function parseScale(): RatingScale | null {
    const min = Number(form.min);
    const max = Number(form.max);
    const step = Number(form.step);
    if (!Number.isFinite(min) || !Number.isFinite(max) || !Number.isFinite(step)) return null;
    if (step <= 0) return null;
    if (max <= min) return null;
    return { min, max, step };
  }

  async function save() {
    formError = undefined;
    const slug = form.slug.trim();
    const name = form.name.trim();
    if (editorMode === 'create' && !slug) { formError = 'Slug is required.'; return; }
    if (!name) { formError = 'Name is required.'; return; }
    const rs = parseScale();
    if (!rs) { formError = 'Invalid rating scale: max must exceed min, step must be positive.'; return; }

    // Model override: provider and model travel together — the server
    // rejects half a pair, so catch it here with a message that says which
    // half is missing.
    const overrideProvider = form.llmProviderId || null;
    const overrideModel = form.llmModel.trim() || null;
    if (overrideProvider && !overrideModel) {
      formError = 'Enter a model for the selected provider (or set provider back to inherit).';
      return;
    }
    if (overrideModel && !overrideProvider) {
      formError = 'Pick a provider for the model (a model alone is not an override).';
      return;
    }
    const overridePool = form.llmPoolId || null;

    saving = true;
    try {
      const llmFields = {
        llm_provider_id: overrideProvider,
        llm_model: overrideModel,
        llm_pool_id: overridePool,
        llm_source_order: form.llmSourceOrder,
        avatar_url: form.avatarUrl,
      };
      if (editorMode === 'create') {
        await createJudge({
          slug,
          name,
          description: form.description,
          prompt: form.prompt,
          rating_scale: rs,
          ...llmFields,
        });
        notify.success('Judge created.', 'Judges');
      } else if (editorMode === 'edit' && editingId !== null) {
        await updateJudge(editingId, {
          name,
          description: form.description,
          prompt: form.prompt,
          rating_scale: rs,
          ...llmFields,
        });
        notify.success('Judge updated.', 'Judges');
      }
      editorMode = null;
      editingId = null;
      await invalidateAll();
    } catch (err) {
      if (err instanceof ApiError) {
        if (err.status === 409) formError = 'Judge slug already exists.';
        else if (err.code === 'validation_error' || err.status === 422) formError = 'Validation failed.';
        else formError = err.code ?? `Error ${err.status}`;
      } else {
        formError = 'Unknown error.';
      }
    } finally {
      saving = false;
    }
  }

  async function onDelete(j: Judge) {
    if (!confirm(`Delete judge "${j.name}"?`)) return;
    deletingId = j.id;
    try {
      await deleteJudge(j.id);
      notify.success(`"${j.name}" deleted.`, 'Judges');
      await invalidateAll();
    } catch (err) {
      if (err instanceof ApiError && err.status === 409) {
        notify.error('Judge is attached to tasks — detach first.', 'Judges');
      } else {
        notify.error('Could not delete judge.', 'Judges');
      }
    } finally {
      deletingId = null;
    }
  }

  function onInput(field: keyof typeof form, e: Event) {
    const target = e.target as HTMLInputElement | HTMLTextAreaElement;
    form = { ...form, [field]: target.value };
  }

  let syncing = $state(false);

  /** Re-run the judge seed against the server's on-disk judges/*.md files. */
  async function onSyncJudges() {
    if (syncing) return;
    syncing = true;
    try {
      const r = await syncJudges({ fetch });
      notify.success(
        `Judges synced — ${r.inserted} added, ${r.updated} refreshed, ${r.skipped} unchanged/skipped`,
        'Judges',
      );
      await invalidateAll();
    } catch (err) {
      notify.error(err instanceof Error ? err.message : String(err), 'Sync failed');
    } finally {
      syncing = false;
    }
  }
</script>

<div class="mt-8">
  <div class="mb-4 flex flex-wrap items-center justify-between gap-3">
    <div>
      <h2 class="font-heading text-[20px] font-semibold text-brand-text">Judges</h2>
      <p class="mt-0.5 text-sm text-brand-muted">
        {judges.length} {judges.length === 1 ? 'judge' : 'judges'} defined.
        Judging prompts used to evaluate task submissions.
      </p>
    </div>
    {#if editorMode === null}
      <div class="flex shrink-0 items-center gap-2">
        <button
          type="button"
          data-testid="sync-judges-btn"
          onclick={onSyncJudges}
          disabled={syncing}
          title="Re-read the server's on-disk judges/*.md definitions: new files insert, changed files overwrite the DB rows"
          class="rounded-btn border border-brand-blue px-5 py-2 text-sm font-semibold text-brand-blue
                 transition-colors hover:bg-brand-blue/10 disabled:opacity-50"
        >
          {syncing ? 'Syncing…' : 'Sync from disk'}
        </button>
        <button
          type="button"
          onclick={startCreate}
          class="rounded-btn bg-brand-blue px-5 py-2 text-sm font-semibold text-white
                 transition-opacity hover:opacity-80"
        >
          New judge
        </button>
      </div>
    {/if}
  </div>

  {#if editorMode !== null}
    <div class="mb-6 rounded-[8px] bg-white p-6 shadow-sm">
      <h3 class="mb-4 font-heading text-[16px] font-semibold text-brand-text">
        {editorMode === 'create' ? 'New judge' : 'Edit judge'}
      </h3>
      <div class="flex flex-col gap-4">
        <div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <div>
            <label class="mb-1 block text-xs font-semibold text-brand-text" for="judge-slug">Slug</label>
            <input
              id="judge-slug"
              type="text"
              value={form.slug}
              disabled={editorMode === 'edit'}
              oninput={(e) => onInput('slug', e)}
              class="w-full rounded-[6px] border border-brand-border px-3 py-2 text-sm text-brand-text
                     focus:outline-none focus:ring-2 focus:ring-brand-blue
                     disabled:bg-gray-50 disabled:text-brand-muted"
            />
            {#if editorMode === 'edit'}
              <p class="mt-1 text-xs text-brand-muted">Slug is immutable after creation.</p>
            {/if}
          </div>
          <div>
            <label class="mb-1 block text-xs font-semibold text-brand-text" for="judge-name">Name</label>
            <input
              id="judge-name"
              type="text"
              value={form.name}
              oninput={(e) => onInput('name', e)}
              class="w-full rounded-[6px] border border-brand-border px-3 py-2 text-sm text-brand-text
                     focus:outline-none focus:ring-2 focus:ring-brand-blue"
            />
          </div>
        </div>

        <div>
          <span class="mb-1 block text-xs font-semibold text-brand-text">Avatar</span>
          <div class="max-w-[200px]">
            <CoverImageUpload value={form.avatarUrl} onchange={(url) => (form = { ...form, avatarUrl: url })} />
          </div>
        </div>

        <div>
          <label class="mb-1 block text-xs font-semibold text-brand-text" for="judge-desc">Description</label>
          <textarea
            id="judge-desc"
            rows="2"
            oninput={(e) => onInput('description', e)}
            class="w-full rounded-[6px] border border-brand-border px-3 py-2 text-sm text-brand-text
                   focus:outline-none focus:ring-2 focus:ring-brand-blue"
          >{form.description}</textarea>
        </div>

        <div>
          <label class="mb-1 block text-xs font-semibold text-brand-text" for="judge-prompt">Prompt</label>
          <textarea
            id="judge-prompt"
            rows="10"
            oninput={(e) => onInput('prompt', e)}
            class="w-full rounded-[6px] border border-brand-border px-3 py-2 font-mono text-sm text-brand-text
                   focus:outline-none focus:ring-2 focus:ring-brand-blue"
          >{form.prompt}</textarea>
          <p class="mt-1 text-xs text-brand-muted">Static prompt — no placeholders supported.</p>
        </div>

        <div class="grid grid-cols-1 gap-4 sm:grid-cols-3">
          <div>
            <label class="mb-1 block text-xs font-semibold text-brand-text" for="judge-min">Min</label>
            <input
              id="judge-min"
              type="number"
              step="0.1"
              value={form.min}
              oninput={(e) => onInput('min', e)}
              class="w-full rounded-[6px] border border-brand-border px-3 py-2 text-sm text-brand-text
                     focus:outline-none focus:ring-2 focus:ring-brand-blue"
            />
          </div>
          <div>
            <label class="mb-1 block text-xs font-semibold text-brand-text" for="judge-max">Max</label>
            <input
              id="judge-max"
              type="number"
              step="0.1"
              value={form.max}
              oninput={(e) => onInput('max', e)}
              class="w-full rounded-[6px] border border-brand-border px-3 py-2 text-sm text-brand-text
                     focus:outline-none focus:ring-2 focus:ring-brand-blue"
            />
          </div>
          <div>
            <label class="mb-1 block text-xs font-semibold text-brand-text" for="judge-step">Step</label>
            <input
              id="judge-step"
              type="number"
              step="0.1"
              value={form.step}
              oninput={(e) => onInput('step', e)}
              class="w-full rounded-[6px] border border-brand-border px-3 py-2 text-sm text-brand-text
                     focus:outline-none focus:ring-2 focus:ring-brand-blue"
            />
          </div>
        </div>

        <div>
          <div class="mb-1 text-xs font-semibold text-brand-text">Model override</div>
          <p class="mb-3 text-xs text-brand-muted">
            Overrides the judge-operation model from
            <a href="/settings/ai" class="text-brand-blue hover:underline">AI settings</a>
            for this judge only. Set a pool, a single model, or both — with both, the one on top
            runs first and the other is its failover.
          </p>

          <div class="flex flex-col gap-4">
            <div>
              <label for="judge-pool" class="mb-1 block text-xs font-semibold text-brand-text">
                Pool
              </label>
              {#if pools.length === 0}
                <p class="text-xs text-brand-muted">
                  No pools defined —
                  <a href="/settings/ai" class="text-brand-blue hover:underline">create one</a>
                  in AI settings.
                </p>
              {:else}
                <select
                  id="judge-pool"
                  value={form.llmPoolId}
                  onchange={(e) => (form.llmPoolId = e.currentTarget.value)}
                  class="w-full max-w-md rounded-[6px] border border-brand-border px-3 py-2 text-sm text-brand-text
                         focus:outline-none focus:ring-2 focus:ring-brand-blue"
                >
                  <option value="">No pool</option>
                  {#each pools as p (p.id)}
                    <option value={p.id}>
                      {p.name} · {p.members.filter((m) => m.enabled).length} models
                    </option>
                  {/each}
                </select>
              {/if}
            </div>

            <div>
              <div class="mb-1 text-xs font-semibold text-brand-text">Single model</div>
              <ModelSelector
                {providers}
                value={form.llmProviderId && form.llmModel
                  ? { provider_id: form.llmProviderId, model: form.llmModel }
                  : null}
                onchange={(sel) => {
                  if (sel) {
                    form.llmProviderId = sel.provider_id;
                    form.llmModel = sel.model;
                  } else {
                    form.llmProviderId = '';
                    form.llmModel = '';
                  }
                }}
                modelsFor={suggestionsFor}
                placeholder="No pinned model…"
                label="Judge model override"
                allowClear={true}
                clearLabel="No pinned model"
              />
            </div>

            <!-- Only meaningful with both halves set; hidden otherwise so it
                 does not imply an ordering that has nothing to order. -->
            {#if form.llmPoolId && form.llmProviderId && form.llmModel}
              <div>
                <label for="judge-order" class="mb-1 block text-xs font-semibold text-brand-text">
                  Which runs first
                </label>
                <select
                  id="judge-order"
                  value={form.llmSourceOrder}
                  onchange={(e) =>
                    (form.llmSourceOrder = e.currentTarget.value as LlmSourceOrder)}
                  class="w-full max-w-md rounded-[6px] border border-brand-border px-3 py-2 text-sm text-brand-text
                         focus:outline-none focus:ring-2 focus:ring-brand-blue"
                >
                  <option value="pool_first">Pool first, then the pinned model</option>
                  <option value="model_first">Pinned model first, then the pool</option>
                </select>
                <p class="mt-1 text-xs text-brand-muted">
                  {form.llmSourceOrder === 'pool_first'
                    ? `Tries the pool's models in order; falls back to ${form.llmModel} if they all fail.`
                    : `Tries ${form.llmModel} first; falls back to the pool if it fails.`}
                </p>
              </div>
            {/if}
          </div>
        </div>

        {#if formError}
          <p class="text-xs text-red-500">{formError}</p>
        {/if}

        <div class="flex justify-end gap-2">
          <button
            type="button"
            onclick={cancelEditor}
            disabled={saving}
            class="rounded-btn border border-brand-border px-5 py-2 text-sm font-semibold text-brand-text
                   transition-opacity hover:opacity-80 disabled:opacity-40"
          >
            Cancel
          </button>
          <button
            type="button"
            onclick={save}
            disabled={saving}
            class="rounded-btn bg-brand-blue px-5 py-2 text-sm font-semibold text-white
                   transition-opacity hover:opacity-80 disabled:opacity-40"
          >
            {saving ? 'Saving…' : 'Save'}
          </button>
        </div>
      </div>
    </div>
  {/if}

  <div class="rounded-[8px] bg-white shadow-sm">
    {#if judges.length === 0}
      <div class="flex flex-col items-center justify-center py-16 text-brand-muted">
        <p class="text-sm">No judges yet. Create one above.</p>
      </div>
    {:else}
      <!-- Cards below `xl`, the same way the sessions registry works: the
           settings column is ~940px wide, which seven columns do not fit. -->
      <ul class="divide-y divide-brand-border/60 xl:hidden">
        {#each judges as j (j.id)}
          {@const usage = usageById.get(j.id)}
          {@const projects = projectsOf(usage)}
          {@const stats = usage?.stats}
          <li class="flex flex-col gap-2.5 p-4">
            <div class="flex items-start justify-between gap-3">
              <span class="flex min-w-0 items-center gap-2.5">
                {#if j.avatar_url}
                  <img
                    src={ikAvatar(j.avatar_url, 28)}
                    alt="{j.name} avatar"
                    class="h-[28px] w-[28px] shrink-0 rounded-full object-cover"
                  />
                {:else}
                  <span
                    class="inline-flex h-[28px] w-[28px] shrink-0 items-center justify-center rounded-full bg-brand-blue/15 text-[12px] font-semibold text-brand-blue"
                  >{j.name.charAt(0).toUpperCase()}</span>
                {/if}
                <span class="flex min-w-0 flex-col leading-tight">
                  <span class="truncate text-sm font-medium text-brand-text">{j.name}</span>
                  <span class="truncate text-xs text-brand-muted">{j.slug}</span>
                </span>
              </span>
              <span class="shrink-0 text-xs text-brand-muted">{scaleSummary(j.rating_scale)}</span>
            </div>

            <dl class="grid grid-cols-[4.5rem_minmax(0,1fr)] items-baseline gap-x-3 gap-y-1.5 text-xs text-brand-muted">
              <dt class="font-semibold uppercase tracking-wider text-brand-muted/70">Attached</dt>
              <dd class="min-w-0">
                {#if projects.length === 0}
                  Not attached
                {:else}
                  <button
                    type="button"
                    onclick={() => toggle(j.id)}
                    data-testid="attachments-toggle-card-{j.slug}"
                    aria-expanded={expanded.has(j.id)}
                    class="text-brand-blue transition-colors hover:underline"
                  >
                    {count(projects.length)}
                    {projects.length === 1 ? 'project' : 'projects'} ·
                    {count(usage?.attachments.length ?? 0)}
                    {(usage?.attachments.length ?? 0) === 1 ? 'task' : 'tasks'}
                    <span class="text-[10px]">{expanded.has(j.id) ? '▲' : '▼'}</span>
                  </button>
                {/if}
              </dd>
              <dt class="font-semibold uppercase tracking-wider text-brand-muted/70">Verdicts</dt>
              <dd class="tabular-nums">
                {#if stats && (stats.verdicts > 0 || stats.failed_runs > 0)}
                  <span class="font-semibold text-brand-text">{count(stats.verdicts)}</span>
                  · {count(stats.players)}
                  {stats.players === 1 ? 'player' : 'players'} · {lastRun(stats.last_verdict_at)}
                  {#if stats.failed_runs > 0}
                    <span class="text-amber-600">· {count(stats.failed_runs)} failed</span>
                  {/if}
                {:else}
                  —
                {/if}
              </dd>
              <dt class="font-semibold uppercase tracking-wider text-brand-muted/70">Points</dt>
              <dd class="tabular-nums">
                {#if stats && stats.verdicts > 0}
                  <span
                    class="font-semibold"
                    class:text-green-600={stats.points_total > 0}
                    class:text-red-500={stats.points_total < 0}
                    class:text-brand-text={stats.points_total === 0}
                  >{signed(stats.points_total)}</span>
                  {#if stats.points_awarded > 0 && stats.points_withdrawn > 0}
                    · +{count(stats.points_awarded)} / −{count(stats.points_withdrawn)}
                  {/if}
                {:else}
                  —
                {/if}
              </dd>
            </dl>

            {#if expanded.has(j.id)}
              <div class="rounded-[6px] bg-brand-light-blue/10 p-3">
                <div class="flex flex-col gap-3" data-testid="attachments-card-{j.slug}">
                  {#each projects as p (p.id)}
                    <div class="text-sm">
                      <a
                        href="/projects/{p.slug ?? p.id}"
                        class="font-medium text-brand-blue hover:underline"
                      >{p.name}</a>
                      <span class="ml-2 text-xs text-brand-muted">
                        {p.tasks.length}
                        {p.tasks.length === 1 ? 'task' : 'tasks'}
                      </span>
                      <ul class="mt-1 flex flex-wrap gap-x-4 gap-y-1">
                        {#each p.tasks as t (t.ordinal)}
                          <li class="text-xs text-brand-muted">
                            <span class="tabular-nums">#{t.ordinal}</span>
                            {t.title}
                          </li>
                        {/each}
                      </ul>
                    </div>
                  {/each}
                </div>
              </div>
            {/if}

            <div class="flex justify-end gap-2">
              <button
                type="button"
                onclick={() => startEdit(j)}
                class="rounded px-3 py-1 text-xs font-semibold text-brand-blue
                       transition-colors hover:bg-brand-blue/10"
              >
                Edit
              </button>
              <button
                type="button"
                onclick={() => onDelete(j)}
                disabled={deletingId === j.id}
                class="rounded px-3 py-1 text-xs font-semibold text-red-500
                       transition-colors hover:bg-red-50 disabled:opacity-40"
              >
                {deletingId === j.id ? 'Deleting…' : 'Delete'}
              </button>
            </div>
          </li>
        {/each}
      </ul>

      <div class="hidden overflow-x-auto xl:block">
      <table class="w-full">
        <thead>
          <tr class="border-b border-brand-border bg-brand-light-blue/40">
            <th class="px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
              Name
            </th>
            <th class="px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
              Slug
            </th>
            <th class="px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
              Scale
            </th>
            <th class="px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
              Attached to
            </th>
            <th class="px-4 py-3 text-right text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
              Verdicts
            </th>
            <th class="px-4 py-3 text-right text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
              Points moved
            </th>
            <th class="px-4 py-3 text-right text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
              Actions
            </th>
          </tr>
        </thead>
        <tbody>
          {#each judges as j (j.id)}
            {@const usage = usageById.get(j.id)}
            {@const projects = projectsOf(usage)}
            {@const stats = usage?.stats}
            <tr class="border-b border-brand-border/60 last:border-0 hover:bg-brand-light-blue/20">
              <td class="px-4 py-4 text-sm font-medium text-brand-text">
                <span class="flex items-center gap-2.5">
                  {#if j.avatar_url}
                    <img
                      src={ikAvatar(j.avatar_url, 28)}
                      alt="{j.name} avatar"
                      class="h-[28px] w-[28px] shrink-0 rounded-full object-cover"
                    />
                  {:else}
                    <span
                      class="inline-flex h-[28px] w-[28px] shrink-0 items-center justify-center rounded-full bg-brand-blue/15 text-[12px] font-semibold text-brand-blue"
                    >{j.name.charAt(0).toUpperCase()}</span>
                  {/if}
                  {j.name}
                </span>
              </td>
              <td class="px-4 py-4 text-sm text-brand-muted">{j.slug}</td>
              <td class="px-4 py-4 text-sm text-brand-muted">{scaleSummary(j.rating_scale)}</td>
              <td class="px-4 py-4 text-sm">
                {#if projects.length === 0}
                  <span class="text-brand-muted">Not attached</span>
                {:else}
                  <button
                    type="button"
                    onclick={() => toggle(j.id)}
                    data-testid="attachments-toggle-{j.slug}"
                    aria-expanded={expanded.has(j.id)}
                    class="text-brand-blue transition-colors hover:underline"
                  >
                    {count(projects.length)}
                    {projects.length === 1 ? 'project' : 'projects'} ·
                    {count(usage?.attachments.length ?? 0)}
                    {(usage?.attachments.length ?? 0) === 1 ? 'task' : 'tasks'}
                    <span class="text-[10px]">{expanded.has(j.id) ? '▲' : '▼'}</span>
                  </button>
                {/if}
              </td>
              <td class="px-4 py-4 text-right text-sm tabular-nums">
                {#if stats && (stats.verdicts > 0 || stats.failed_runs > 0)}
                  <span class="font-semibold text-brand-text">{count(stats.verdicts)}</span>
                  <span class="block text-[11px] text-brand-muted">
                    {count(stats.players)}
                    {stats.players === 1 ? 'player' : 'players'} · {lastRun(stats.last_verdict_at)}
                  </span>
                  {#if stats.failed_runs > 0}
                    <span class="block text-[11px] text-amber-600">
                      {count(stats.failed_runs)} failed
                    </span>
                  {/if}
                {:else}
                  <span class="text-brand-muted">—</span>
                {/if}
              </td>
              <td class="px-4 py-4 text-right text-sm tabular-nums">
                {#if stats && stats.verdicts > 0}
                  <span
                    class="font-semibold"
                    class:text-green-600={stats.points_total > 0}
                    class:text-red-500={stats.points_total < 0}
                    class:text-brand-text={stats.points_total === 0}
                  >{signed(stats.points_total)}</span>
                  {#if stats.points_awarded > 0 && stats.points_withdrawn > 0}
                    <span class="block text-[11px] text-brand-muted">
                      +{count(stats.points_awarded)} / −{count(stats.points_withdrawn)}
                    </span>
                  {/if}
                {:else}
                  <span class="text-brand-muted">—</span>
                {/if}
              </td>
              <td class="px-4 py-4 text-right">
                <div class="flex justify-end gap-2">
                  <button
                    type="button"
                    onclick={() => startEdit(j)}
                    class="rounded px-3 py-1 text-xs font-semibold text-brand-blue
                           transition-colors hover:bg-brand-blue/10"
                  >
                    Edit
                  </button>
                  <button
                    type="button"
                    onclick={() => onDelete(j)}
                    disabled={deletingId === j.id}
                    class="rounded px-3 py-1 text-xs font-semibold text-red-500
                           transition-colors hover:bg-red-50 disabled:opacity-40"
                  >
                    {deletingId === j.id ? 'Deleting…' : 'Delete'}
                  </button>
                </div>
              </td>
            </tr>
            {#if expanded.has(j.id)}
              <tr class="border-b border-brand-border/60 bg-brand-light-blue/10">
                <td colspan="7" class="px-4 py-4">
                  <div class="flex flex-col gap-3" data-testid="attachments-{j.slug}">
                    {#each projects as p (p.id)}
                      <div class="text-sm">
                        <a
                          href="/projects/{p.slug ?? p.id}"
                          class="font-medium text-brand-blue hover:underline"
                        >{p.name}</a>
                        <span class="ml-2 text-xs text-brand-muted">
                          {p.tasks.length}
                          {p.tasks.length === 1 ? 'task' : 'tasks'}
                        </span>
                        <ul class="mt-1 flex flex-wrap gap-x-4 gap-y-1">
                          {#each p.tasks as t (t.ordinal)}
                            <li class="text-xs text-brand-muted">
                              <span class="tabular-nums">#{t.ordinal}</span>
                              {t.title}
                            </li>
                          {/each}
                        </ul>
                      </div>
                    {/each}
                  </div>
                </td>
              </tr>
            {/if}
          {/each}
        </tbody>
      </table>
      </div>
    {/if}
  </div>
</div>