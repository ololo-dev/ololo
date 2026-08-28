<script lang="ts">
  import { invalidateAll } from '$app/navigation';
  import { untrack } from 'svelte';
  import { notify } from '$lib/notifications.svelte';
  import {
    createLlmProvider,
    updateLlmProvider,
    deleteLlmProvider,
    testLlmProvider,
    listLlmProviderModels,
    updateLlmAssignments,
    updateJudge,
    createLlmPool,
    updateLlmPool,
    deleteLlmPool,
    isPoolAssignment,
    ApiError,
    type Judge,
    type LlmProvider,
    type LlmProviderKind,
    type LlmAssignment,
    type LlmAssignments,
    type LlmOperation,
    type LlmPool,
    type LlmPoolMemberInput,
    type CreateLlmProviderBody,
    type UpdateLlmProviderBody,
  } from '$lib/api';
  import { loadModelsDevCatalog, providerLogoId, type ModelsDevProvider } from '$lib/models-dev';
  import { modelSuggestions } from '$lib/llm-models';
  import AssignmentSelector from '$lib/components/AssignmentSelector.svelte';
  import ModelSelector from '$lib/components/ModelSelector.svelte';
  import ProviderIcon from '$lib/components/ProviderIcon.svelte';

  let { data } = $props();

  let providers = $state<LlmProvider[]>(untrack(() => [...data.providers]));
  $effect(() => { providers = [...data.providers]; });

  let assignments = $state<LlmAssignments>(untrack(() => cloneAssignments(data.assignments)));
  $effect(() => { assignments = cloneAssignments(data.assignments); });

  let judges = $state<Judge[]>(untrack(() => [...data.judges]));
  $effect(() => { judges = [...data.judges]; });

  let pools = $state<LlmPool[]>(untrack(() => [...data.pools]));
  $effect(() => { pools = [...data.pools]; });

  function cloneAssignments(a: LlmAssignments): LlmAssignments {
    return {
      default: a.default ? { ...a.default } : null,
      operations: { ...a.operations },
    };
  }

  const KIND_LABELS: Record<LlmProviderKind, string> = {
    ollama: 'Ollama',
    openrouter: 'OpenRouter',
    openai_compatible: 'OpenAI-compatible',
  };

  const OPERATIONS: { key: LlmOperation; label: string }[] = [
    { key: 'judge', label: 'Judges (default)' },
    { key: 'memory', label: 'Memory extraction' },
    { key: 'adaptation', label: 'Task adaptation' },
    { key: 'project_ai', label: 'Project & task AI' },
  ];

  // ---------- Provider editor ----------

  type EditorMode = 'create' | 'edit' | null;
  let editorMode = $state<EditorMode>(null);

  const emptyForm = (): {
    name: string;
    kind: LlmProviderKind;
    base_url: string;
    api_key: string;
    remove_key: boolean;
    enabled: boolean;
    catalog_id: string;
  } => ({
    name: '',
    kind: 'openai_compatible',
    base_url: '',
    api_key: '',
    remove_key: false,
    enabled: true,
    catalog_id: '',
  });

  let form = $state(untrack(emptyForm));
  let editingId = $state<string | null>(null);
  let editingHasKey = $state(false);
  let saving = $state(false);
  let formError = $state<string | undefined>(undefined);
  let deletingId = $state<string | null>(null);
  let togglingId = $state<string | null>(null);

  function startCreate() {
    editorMode = 'create';
    editingId = null;
    editingHasKey = false;
    form = emptyForm();
    formError = undefined;
    catalogOpen = false;
    pickedCatalog = null;
  }

  function startEdit(p: LlmProvider) {
    editorMode = 'edit';
    editingId = p.id;
    editingHasKey = p.has_api_key;
    form = {
      name: p.name,
      kind: p.kind,
      base_url: p.base_url ?? '',
      api_key: '',
      remove_key: false,
      enabled: p.enabled,
      catalog_id: p.catalog_id ?? '',
    };
    formError = undefined;
    catalogOpen = false;
    pickedCatalog = null;
  }

  function cancelEditor() {
    editorMode = null;
    editingId = null;
    formError = undefined;
    catalogOpen = false;
    pickedCatalog = null;
  }

  function errorText(err: unknown): string {
    if (err instanceof ApiError) {
      if (err.code === 'validation_error' || err.status === 422) return 'Validation failed.';
      return err.code ?? `Error ${err.status}`;
    }
    return 'Unknown error.';
  }

  async function saveProvider() {
    formError = undefined;
    const name = form.name.trim();
    const baseUrl = form.base_url.trim();
    if (!name) { formError = 'Name is required.'; return; }
    if (form.kind === 'openai_compatible' && !baseUrl) {
      formError = 'Base URL is required for OpenAI-compatible providers.';
      return;
    }

    saving = true;
    try {
      if (editorMode === 'create') {
        const body: CreateLlmProviderBody = { name, kind: form.kind, enabled: form.enabled };
        if (baseUrl) body.base_url = baseUrl;
        if (form.api_key) body.api_key = form.api_key;
        if (form.catalog_id) body.catalog_id = form.catalog_id;
        await createLlmProvider(body);
        notify.success('Provider created.', 'AI');
      } else if (editorMode === 'edit' && editingId !== null) {
        const body: UpdateLlmProviderBody = { name, kind: form.kind, enabled: form.enabled };
        if (baseUrl) body.base_url = baseUrl;
        if (form.remove_key) body.clear_api_key = true;
        else if (form.api_key) body.api_key = form.api_key;
        if (form.catalog_id) body.catalog_id = form.catalog_id;
        await updateLlmProvider(editingId, body);
        notify.success('Provider updated.', 'AI');
      }
      editorMode = null;
      editingId = null;
      await invalidateAll();
    } catch (err) {
      formError = errorText(err);
    } finally {
      saving = false;
    }
  }

  async function toggleEnabled(p: LlmProvider) {
    togglingId = p.id;
    try {
      await updateLlmProvider(p.id, { enabled: !p.enabled });
      await invalidateAll();
    } catch {
      notify.error('Could not update provider.', 'AI');
    } finally {
      togglingId = null;
    }
  }

  // ---------- Provider connectivity test ----------
  // One real completion through the stored key; results keyed by provider id.
  type ProviderTest = {
    running: boolean;
    model: string;
    ok?: boolean;
    latency_ms?: number;
    output?: string;
    error?: string;
  };
  let providerTests = $state<Record<string, ProviderTest>>({});

  // Model suggestions for the test block's datalist, cached per provider.
  // A native datalist survives the table's overflow container, which would
  // clip a custom dropdown.
  let testModelOptions = $state<Record<string, string[]>>({});
  function ensureTestModelOptions(p: LlmProvider) {
    if (testModelOptions[p.id]) return;
    void suggestionsFor(p.id).then((models) => {
      testModelOptions = { ...testModelOptions, [p.id]: models };
    });
  }

  async function runProviderTest(p: LlmProvider) {
    ensureTestModelOptions(p);
    let model = (providerTests[p.id]?.model ?? '').trim();
    providerTests = { ...providerTests, [p.id]: { running: true, model } };
    try {
      if (!model) {
        // No model chosen yet: default to the first one the provider lists.
        const models = await listLlmProviderModels(p.id);
        model = models[0] ?? '';
        if (!model) {
          providerTests = {
            ...providerTests,
            [p.id]: {
              running: false,
              model: '',
              ok: false,
              error: 'The provider listed no models — type a model id and run again.',
            },
          };
          return;
        }
      }
      const r = await testLlmProvider(p.id, { model });
      providerTests = {
        ...providerTests,
        [p.id]: {
          running: false,
          model,
          ok: r.ok,
          latency_ms: r.latency_ms,
          output: r.output ?? undefined,
          error: r.error ?? undefined,
        },
      };
    } catch (err) {
      providerTests = {
        ...providerTests,
        [p.id]: { running: false, model, ok: false, error: errorText(err) },
      };
    }
  }

  function setTestModel(p: LlmProvider, model: string) {
    const prev = providerTests[p.id] ?? { running: false, model: '' };
    providerTests = { ...providerTests, [p.id]: { ...prev, model } };
  }

  function dismissProviderTest(p: LlmProvider) {
    const next = { ...providerTests };
    delete next[p.id];
    providerTests = next;
  }

  async function onDeleteProvider(p: LlmProvider) {
    if (!confirm(`Delete provider "${p.name}"? Assignments and judge overrides using it will be cleared.`)) return;
    deletingId = p.id;
    try {
      await deleteLlmProvider(p.id);
      notify.success(`"${p.name}" deleted.`, 'AI');
      await invalidateAll();
    } catch {
      notify.error('Could not delete provider.', 'AI');
    } finally {
      deletingId = null;
    }
  }

  // ---------- models.dev catalog assist ----------

  let catalogOpen = $state(false);
  let catalogQuery = $state('');
  let catalogProviders = $state<ModelsDevProvider[] | null>(null);
  let catalogLoading = $state(false);
  let catalogError = $state<string | undefined>(undefined);

  async function toggleCatalog() {
    catalogOpen = !catalogOpen;
    if (!catalogOpen || catalogProviders !== null || catalogLoading) return;
    catalogLoading = true;
    catalogError = undefined;
    try {
      catalogProviders = await loadModelsDevCatalog();
    } catch {
      catalogError = 'Could not load the models.dev catalog — enter provider details manually.';
    } finally {
      catalogLoading = false;
    }
  }

  const filteredCatalog = $derived.by(() => {
    if (!catalogProviders) return [];
    const q = catalogQuery.trim().toLowerCase();
    const list = q
      ? catalogProviders.filter(
          (c) => c.id.toLowerCase().includes(q) || c.name.toLowerCase().includes(q),
        )
      : catalogProviders;
    return list.slice(0, 30);
  });

  /**
   * Ollama is the only provider that is not just an OpenAI-compatible
   * endpoint: it runs locally and its API key is optional. Everything else,
   * OpenRouter included, is reached through the same client with a base URL.
   */
  function catalogKind(id: string): LlmProviderKind {
    return id === 'ollama' ? 'ollama' : 'openai_compatible';
  }

  /** The catalog entry the form was filled from, to explain a blank base URL. */
  let pickedCatalog = $state<ModelsDevProvider | null>(null);

  function pickCatalog(c: ModelsDevProvider) {
    form = {
      ...form,
      name: c.name,
      kind: catalogKind(c.id),
      base_url: c.api ?? '',
      catalog_id: c.id,
    };
    pickedCatalog = c;
    catalogOpen = false;
    catalogQuery = '';
  }

  // ---------- Model suggestions (live models + catalog models) ----------

  /** Suggestion loader for `ModelSelector`; cached per provider in $lib/llm-models. */
  function suggestionsFor(providerId: string): Promise<string[]> {
    const p = providers.find((x) => x.id === providerId);
    return p ? modelSuggestions(p) : Promise.resolve([]);
  }

  // ---------- Pool editor ----------

  /** A member row being edited. `priority` is a string so the number input
   *  can be cleared mid-typing without snapping back to 0. */
  type MemberDraft = { provider_id: string; model: string; priority: string; enabled: boolean };

  let poolEditor = $state<'create' | 'edit' | null>(null);
  let poolEditingId = $state<string | null>(null);
  let poolName = $state('');
  let poolDescription = $state('');
  let poolMembers = $state<MemberDraft[]>([]);
  let poolError = $state<string | undefined>(undefined);
  let poolSaving = $state(false);
  let poolDeletingId = $state<string | null>(null);

  function startCreatePool() {
    poolEditor = 'create';
    poolEditingId = null;
    poolName = '';
    poolDescription = '';
    poolMembers = [];
    poolError = undefined;
  }

  function startEditPool(p: LlmPool) {
    poolEditor = 'edit';
    poolEditingId = p.id;
    poolName = p.name;
    poolDescription = p.description;
    poolMembers = p.members.map((m) => ({
      provider_id: m.provider_id,
      model: m.model,
      priority: String(m.priority),
      enabled: m.enabled,
    }));
    poolError = undefined;
  }

  function cancelPoolEditor() {
    poolEditor = null;
    poolEditingId = null;
    poolError = undefined;
  }

  function addMember() {
    // New rows join the last tier by default — the common case is widening
    // an existing tier rather than adding a deeper fallback.
    const last = poolMembers.at(-1);
    poolMembers = [
      ...poolMembers,
      {
        provider_id: providers.find((p) => p.enabled)?.id ?? '',
        model: '',
        priority: last?.priority ?? '0',
        enabled: true,
      },
    ];
  }

  function removeMember(i: number) {
    poolMembers = poolMembers.filter((_, idx) => idx !== i);
  }

  function patchMember(i: number, patch: Partial<MemberDraft>) {
    poolMembers = poolMembers.map((m, idx) => (idx === i ? { ...m, ...patch } : m));
  }

  /** Members grouped by tier, for the "what runs first" summary. */
  const poolPreview = $derived.by(() => {
    const tiers = new Map<number, string[]>();
    for (const m of poolMembers) {
      if (!m.enabled || !m.model.trim()) continue;
      const pri = Number(m.priority) || 0;
      const label = `${providers.find((p) => p.id === m.provider_id)?.name ?? '?'} / ${m.model.trim()}`;
      tiers.set(pri, [...(tiers.get(pri) ?? []), label]);
    }
    return [...tiers.entries()].sort((a, b) => a[0] - b[0]);
  });

  async function savePool() {
    poolError = undefined;
    const name = poolName.trim();
    if (!name) { poolError = 'Name is required.'; return; }
    for (const m of poolMembers) {
      if (!m.provider_id) { poolError = 'Every member needs a provider.'; return; }
      if (!m.model.trim()) { poolError = 'Every member needs a model.'; return; }
    }
    const members: LlmPoolMemberInput[] = poolMembers.map((m) => ({
      provider_id: m.provider_id,
      model: m.model.trim(),
      priority: Number(m.priority) || 0,
      enabled: m.enabled,
    }));

    poolSaving = true;
    try {
      if (poolEditor === 'create') {
        await createLlmPool({ name, description: poolDescription.trim(), members });
        notify.success('Pool created.', 'AI');
      } else if (poolEditingId !== null) {
        await updateLlmPool(poolEditingId, {
          name,
          description: poolDescription.trim(),
          members,
        });
        notify.success('Pool updated.', 'AI');
      }
      poolEditor = null;
      poolEditingId = null;
      await invalidateAll();
    } catch (err) {
      poolError =
        err instanceof ApiError && err.code === 'name_conflict'
          ? 'A pool with that name already exists.'
          : errorText(err);
    } finally {
      poolSaving = false;
    }
  }

  async function onDeletePool(p: LlmPool) {
    if (!confirm(`Delete pool "${p.name}"? Assignments using it will be cleared.`)) return;
    poolDeletingId = p.id;
    try {
      await deleteLlmPool(p.id);
      notify.success(`"${p.name}" deleted.`, 'AI');
      await invalidateAll();
    } catch {
      notify.error('Could not delete pool.', 'AI');
    } finally {
      poolDeletingId = null;
    }
  }

  function tierSummary(p: LlmPool): string {
    const tiers = new Set(p.members.filter((m) => m.enabled).map((m) => m.priority));
    const active = p.members.filter((m) => m.enabled).length;
    if (active === 0) return 'no active members';
    const t = tiers.size === 1 ? '1 tier' : `${tiers.size} tiers`;
    return `${active === 1 ? '1 model' : `${active} models`} · ${t}`;
  }

  // ---------- Assignments (default + per-operation) ----------

  /**
   * The staged selection for one row. `null` means "nothing chosen" — either
   * the row inherits, or the admin cleared it and has not picked a
   * replacement yet.
   */
  type AssignmentForm = LlmAssignment | null;

  function toForm(a: LlmAssignment | null | undefined): AssignmentForm {
    return a ?? null;
  }

  function buildOpForms(a: LlmAssignments): Record<LlmOperation, AssignmentForm> {
    const out = {} as Record<LlmOperation, AssignmentForm>;
    for (const { key } of OPERATIONS) out[key] = toForm(a.operations[key]);
    return out;
  }

  let defaultForm = $state<AssignmentForm>(untrack(() => toForm(data.assignments.default)));
  $effect(() => { defaultForm = toForm(data.assignments.default); });

  let opForms = $state<Record<LlmOperation, AssignmentForm>>(
    untrack(() => buildOpForms(data.assignments)),
  );
  $effect(() => { opForms = buildOpForms(data.assignments); });

  function buildJudgeForms(list: Judge[]): Record<string, AssignmentForm> {
    const out: Record<string, AssignmentForm> = {};
    for (const j of list) out[j.id] = judgeStored(j);
    return out;
  }

  let judgeForms = $state<Record<string, AssignmentForm>>(
    untrack(() => buildJudgeForms(data.judges)),
  );
  $effect(() => { judgeForms = buildJudgeForms(data.judges); });

  /** Key of the row currently being saved: 'default', 'op:<op>' or 'judge:<id>'. */
  let busyKey = $state<string | null>(null);

  function assignmentComplete(f: AssignmentForm): boolean {
    if (!f) return false;
    return isPoolAssignment(f) ? f.pool_id !== '' : f.provider_id !== '' && f.model.trim() !== '';
  }

  /** Compare by serialized shape — the two variants have no common fields. */
  function assignmentKey(a: LlmAssignment | null | undefined): string {
    if (!a) return '';
    return isPoolAssignment(a) ? `pool:${a.pool_id}` : `model:${a.provider_id}:${a.model.trim()}`;
  }

  function assignmentDirty(f: AssignmentForm, stored: LlmAssignment | null | undefined): boolean {
    return assignmentKey(f) !== assignmentKey(stored);
  }

  /** Payload for a save — trims the model id on the single-model variant. */
  function toPayload(f: AssignmentForm): LlmAssignment | null {
    if (!f) return null;
    return isPoolAssignment(f)
      ? { pool_id: f.pool_id }
      : { provider_id: f.provider_id, model: f.model.trim() };
  }

  async function saveDefault() {
    busyKey = 'default';
    try {
      await updateLlmAssignments({ default: toPayload(defaultForm) });
      notify.success('Default model saved.', 'AI');
      await invalidateAll();
    } catch (err) {
      notify.error(errorText(err), 'AI');
    } finally {
      busyKey = null;
    }
  }

  async function clearDefault() {
    busyKey = 'default';
    try {
      await updateLlmAssignments({ default: null });
      notify.success('Default model cleared.', 'AI');
      await invalidateAll();
    } catch (err) {
      notify.error(errorText(err), 'AI');
    } finally {
      busyKey = null;
    }
  }

  async function saveOperation(op: LlmOperation) {
    const f = opForms[op];
    busyKey = `op:${op}`;
    try {
      await updateLlmAssignments({ operations: { [op]: toPayload(f) } });
      notify.success('Operation assignment saved.', 'AI');
      await invalidateAll();
    } catch (err) {
      notify.error(errorText(err), 'AI');
    } finally {
      busyKey = null;
    }
  }

  /**
   * The default is the fallback for every operation that has none of its own,
   * so removing it can silently stop AI features that were working. Unlike
   * the per-row clears, which only revert to a still-working default, this
   * one asks first.
   */
  function confirmClearDefault() {
    const orphaned = OPERATIONS.filter((o) => !assignments.operations[o.key]).length;
    const detail =
      orphaned > 0
        ? `${orphaned} operation${orphaned === 1 ? '' : 's'} rely on it and will have no model.`
        : 'Every operation has its own assignment, so none are affected.';
    if (!confirm(`Remove the default model?\n\n${detail}`)) return;
    void clearDefault();
  }

  async function clearOperation(op: LlmOperation) {
    busyKey = `op:${op}`;
    try {
      await updateLlmAssignments({ operations: { [op]: null } });
      notify.success('Operation now inherits the default model.', 'AI');
      await invalidateAll();
    } catch (err) {
      notify.error(errorText(err), 'AI');
    } finally {
      busyKey = null;
    }
  }

  /**
   * A judge may pin a pool *and* a model. This table edits whichever single
   * source it currently uses; a judge with both is edited on the judges page,
   * which is the only place the ordering between them can be set.
   */
  function judgeStored(j: Judge): LlmAssignment | null {
    if (j.llm_pool_id && !j.llm_model) return { pool_id: j.llm_pool_id };
    if (j.llm_provider_id && j.llm_model)
      return { provider_id: j.llm_provider_id, model: j.llm_model };
    return null;
  }

  function judgeHasOverride(j: Judge): boolean {
    return j.llm_provider_id !== null || j.llm_model !== null || j.llm_pool_id !== null;
  }

  /** True when the judge pins both halves — this table cannot express that. */
  function judgeHasBoth(j: Judge): boolean {
    return j.llm_pool_id !== null && j.llm_model !== null;
  }

  async function saveJudgeOverride(j: Judge) {
    const f = judgeForms[j.id];
    if (!f) return;
    busyKey = `judge:${j.id}`;
    try {
      // Whichever half is not chosen is cleared, so the row always reflects
      // exactly what the selector shows.
      await updateJudge(
        j.id,
        isPoolAssignment(f)
          ? { llm_pool_id: f.pool_id, llm_provider_id: null, llm_model: null }
          : { llm_pool_id: null, llm_provider_id: f.provider_id, llm_model: f.model.trim() },
      );
      notify.success(`Model override saved for "${j.name}".`, 'AI');
      await invalidateAll();
    } catch (err) {
      notify.error(errorText(err), 'AI');
    } finally {
      busyKey = null;
    }
  }

  async function clearJudgeOverride(j: Judge) {
    busyKey = `judge:${j.id}`;
    try {
      await updateJudge(j.id, {
        llm_provider_id: null,
        llm_model: null,
        llm_pool_id: null,
      });
      notify.success(`"${j.name}" now inherits the judge default.`, 'AI');
      await invalidateAll();
    } catch (err) {
      notify.error(errorText(err), 'AI');
    } finally {
      busyKey = null;
    }
  }

  function onFormInput(field: 'name' | 'base_url' | 'api_key' | 'catalog_id', e: Event) {
    const target = e.target as HTMLInputElement;
    form = { ...form, [field]: target.value };
  }

  // ---------- AssignmentSelector wiring ----------

  function onDefaultChange(sel: LlmAssignment | null) {
    defaultForm = sel;
    // Clearing the selector is the clear action itself — there is nothing
    // left to press Save on.
    if (!sel && assignments.default) void clearDefault();
  }

  function onOpChange(op: LlmOperation, sel: LlmAssignment | null) {
    opForms[op] = sel;
    if (!sel && assignments.operations[op]) void clearOperation(op);
  }

  function onJudgeChange(j: Judge, sel: LlmAssignment | null) {
    judgeForms[j.id] = sel;
    if (!sel && judgeHasOverride(j)) void clearJudgeOverride(j);
  }
</script>

<div class="mt-8 flex flex-col gap-10">

  <!-- ============ Providers ============ -->
  <section>
    <div class="mb-4 flex flex-wrap items-start justify-between gap-4">
      <div>
        <h2 class="font-heading text-[20px] font-semibold text-brand-text">Providers</h2>
        <p class="mt-0.5 text-sm text-brand-muted">
          {providers.length} {providers.length === 1 ? 'provider' : 'providers'} configured.
          LLM endpoints used for judging, memory, adaptation and project AI.
        </p>
      </div>
      {#if editorMode === null}
        <button
          type="button"
          onclick={startCreate}
          class="shrink-0 rounded-btn bg-brand-blue px-5 py-2 text-sm font-semibold text-white
                 transition-opacity hover:opacity-80"
        >
          New provider
        </button>
      {/if}
    </div>

    {#if editorMode !== null}
      <div class="mb-6 rounded-[8px] bg-white p-6 shadow-sm">
        <div class="mb-4 flex items-center justify-between">
          <h3 class="font-heading text-[16px] font-semibold text-brand-text">
            {editorMode === 'create' ? 'New provider' : 'Edit provider'}
          </h3>
          <button
            type="button"
            onclick={toggleCatalog}
            class="rounded px-3 py-1 text-xs font-semibold text-brand-blue
                   transition-colors hover:bg-brand-blue/10"
          >
            Choose from catalog…
          </button>
        </div>

        {#if catalogOpen}
          <div class="mb-4 rounded-[6px] border border-brand-border p-3">
            {#if catalogLoading}
              <p class="text-sm text-brand-muted">Loading models.dev catalog…</p>
            {:else if catalogError}
              <p class="text-sm text-red-500">{catalogError}</p>
            {:else}
              <input
                type="text"
                placeholder="Search providers…"
                aria-label="Search catalog providers"
                value={catalogQuery}
                oninput={(e) => { catalogQuery = e.currentTarget.value; }}
                class="mb-2 w-full rounded-[6px] border border-brand-border px-3 py-2 text-sm text-brand-text
                       focus:outline-none focus:ring-2 focus:ring-brand-blue"
              />
              {#if filteredCatalog.length === 0}
                <p class="text-sm text-brand-muted">No matching catalog providers.</p>
              {:else}
                <ul class="max-h-56 overflow-y-auto">
                  {#each filteredCatalog as c (c.id)}
                    <li>
                      <!-- Show the endpoint that will be filled in: picking a
                           provider blind and then hitting "Base URL is
                           required" gave no clue that the catalog simply has
                           no URL for it. -->
                      <button
                        type="button"
                        onclick={() => pickCatalog(c)}
                        class="w-full rounded px-2 py-1.5 text-left text-sm text-brand-text
                               transition-colors hover:bg-brand-light-blue/60"
                      >
                        <span class="flex items-baseline justify-between gap-2">
                          <span class="font-medium">{c.name}</span>
                          <span class="shrink-0 text-xs text-brand-muted">
                            {c.models.length}
                            {c.models.length === 1 ? 'model' : 'models'}
                          </span>
                        </span>
                        <span class="mt-0.5 block truncate font-mono text-[11px] text-brand-muted">
                          {#if c.api}
                            {c.api}
                          {:else}
                            no endpoint in catalog — enter the base URL yourself
                          {/if}
                        </span>
                      </button>
                    </li>
                  {/each}
                </ul>
              {/if}
            {/if}
          </div>
        {/if}

        <div class="flex flex-col gap-4">
          <div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
            <div>
              <label class="mb-1 block text-xs font-semibold text-brand-text" for="prov-name">Name</label>
              <input
                id="prov-name"
                type="text"
                value={form.name}
                oninput={(e) => onFormInput('name', e)}
                class="w-full rounded-[6px] border border-brand-border px-3 py-2 text-sm text-brand-text
                       focus:outline-none focus:ring-2 focus:ring-brand-blue"
              />
            </div>
            <div>
              <label class="mb-1 block text-xs font-semibold text-brand-text" for="prov-kind">Kind</label>
              <select
                id="prov-kind"
                value={form.kind}
                onchange={(e) => { form = { ...form, kind: e.currentTarget.value as LlmProviderKind }; }}
                class="w-full rounded-[6px] border border-brand-border px-3 py-2 text-sm text-brand-text
                       focus:outline-none focus:ring-2 focus:ring-brand-blue"
              >
                <option value="openai_compatible">OpenAI-compatible</option>
                <option value="ollama">Ollama</option>
                <!-- OpenRouter is an OpenAI-compatible endpoint like any
                     other, so it is no longer offered as its own kind. Rows
                     already saved as one keep working and keep showing it,
                     rather than being silently rewritten on the next save. -->
                {#if form.kind === 'openrouter'}
                  <option value="openrouter">OpenRouter (legacy)</option>
                {/if}
              </select>
              <p class="mt-1 text-xs text-brand-muted">
                Almost everything speaks the OpenAI API. Pick Ollama only for a local
                daemon — it is the one kind whose API key is optional.
              </p>
            </div>
          </div>

          <div>
            <label class="mb-1 block text-xs font-semibold text-brand-text" for="prov-base-url">
              Base URL{form.kind === 'openai_compatible' ? '' : ' (optional)'}
            </label>
            <input
              id="prov-base-url"
              type="text"
              placeholder="https://api.example.com/v1"
              value={form.base_url}
              oninput={(e) => onFormInput('base_url', e)}
              class="w-full rounded-[6px] border border-brand-border px-3 py-2 text-sm text-brand-text
                     focus:outline-none focus:ring-2 focus:ring-brand-blue"
            />
            {#if pickedCatalog && !pickedCatalog.api}
              <!-- Picked from the catalog but it had no endpoint: say so and
                   point at the provider's docs, instead of letting the save
                   fail with a bare "Base URL is required". -->
              <p class="mt-1 text-xs text-amber-700">
                models.dev lists no endpoint for {pickedCatalog.name} — paste its
                OpenAI-compatible base URL here{#if pickedCatalog.doc}{' '}(see
                  <a
                    href={pickedCatalog.doc}
                    target="_blank"
                    rel="noopener noreferrer"
                    class="font-semibold text-brand-blue hover:underline">their docs</a
                  >){/if}.
              </p>
            {:else if pickedCatalog?.apiFromFallback}
              <p class="mt-1 text-xs text-brand-muted">
                models.dev has no endpoint for {pickedCatalog.name}; this is its known
                OpenAI-compatible URL. Adjust it if your account uses a different one.
              </p>
            {:else if form.kind !== 'openai_compatible'}
              <p class="mt-1 text-xs text-brand-muted">Leave blank to use the provider's default endpoint.</p>
            {/if}
          </div>

          <div>
            <label class="mb-1 block text-xs font-semibold text-brand-text" for="prov-api-key">API key</label>
            <input
              id="prov-api-key"
              type="password"
              autocomplete="off"
              disabled={form.remove_key}
              value={form.api_key}
              oninput={(e) => onFormInput('api_key', e)}
              class="w-full rounded-[6px] border border-brand-border px-3 py-2 text-sm text-brand-text
                     focus:outline-none focus:ring-2 focus:ring-brand-blue
                     disabled:bg-gray-50 disabled:text-brand-muted"
            />
            {#if editorMode === 'edit' && editingHasKey}
              <div class="mt-1 flex items-center gap-2">
                <p class="text-xs text-brand-muted">
                  A key is stored — leave blank to keep, or tick 'remove key'.
                </p>
                <label class="flex items-center gap-1 text-xs font-semibold text-brand-text">
                  <input
                    type="checkbox"
                    checked={form.remove_key}
                    onchange={(e) => { form = { ...form, remove_key: e.currentTarget.checked }; }}
                  />
                  remove key
                </label>
              </div>
            {/if}
          </div>

          <label class="flex items-center gap-2 text-sm font-semibold text-brand-text">
            <input
              type="checkbox"
              checked={form.enabled}
              onchange={(e) => { form = { ...form, enabled: e.currentTarget.checked }; }}
            />
            Enabled
          </label>

          {#if form.catalog_id}
            <p class="text-xs text-brand-muted">Linked to models.dev catalog entry: <code>{form.catalog_id}</code></p>
          {/if}

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
              onclick={saveProvider}
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

    <!-- The settings layout caps this column at ~940px (1206px page, minus the
         212px sidebar and padding), which the table only clears on a wide
         viewport. Rather than let it squeeze — that is what wrapped the status
         labels onto two lines — the same rows render as cards below `xl`. Both
         layouts share their cell markup via snippets so they cannot drift. -->
    {#snippet providerIdentity(p: LlmProvider)}
      <!-- Kind rides along under the name rather than owning a column: it is
           the least varied field (mostly a repeated "OpenAI-compatible") and
           dropping the column is what lets the other five fit the ~940px the
           settings layout actually gives this page. -->
      <span class="flex min-w-0 items-center gap-2">
        <ProviderIcon name={p.name} catalogId={providerLogoId(p)} size={20} />
        <span class="flex min-w-0 flex-col leading-tight">
          <span class="flex min-w-0 items-baseline gap-1.5">
            <span class="truncate text-sm font-medium text-brand-text">{p.name}</span>
            {#if p.catalog_id}
              <span class="shrink-0 text-xs font-normal text-brand-muted">({p.catalog_id})</span>
            {/if}
          </span>
          <span class="text-xs text-brand-muted">{KIND_LABELS[p.kind]}</span>
        </span>
      </span>
    {/snippet}

    {#snippet keyState(p: LlmProvider)}
      <!-- A dot + plain word instead of a coloured pill: two pills side by
           side read as two competing states, and "No key" on an Ollama row
           looked like a misconfiguration when a key is simply not needed. -->
      <span class="flex items-center gap-1.5 whitespace-nowrap text-brand-muted">
        <span
          class="h-1.5 w-1.5 shrink-0 rounded-full
                 {p.has_api_key ? 'bg-green-500' : 'bg-brand-border'}"
        ></span>
        {#if p.has_api_key}
          Set
        {:else if p.kind === 'ollama'}
          Not needed
        {:else}
          None
        {/if}
      </span>
    {/snippet}

    {#snippet enabledToggle(p: LlmProvider)}
      <button
        type="button"
        onclick={() => toggleEnabled(p)}
        disabled={togglingId === p.id}
        title={p.enabled ? 'Click to disable' : 'Click to enable'}
        class="shrink-0 rounded-full px-2.5 py-0.5 text-[11px] font-semibold transition-opacity
               hover:opacity-80 disabled:opacity-40
               {p.enabled ? 'bg-brand-blue/10 text-brand-blue' : 'bg-gray-100 text-brand-muted'}"
      >
        {p.enabled ? 'Enabled' : 'Disabled'}
      </button>
    {/snippet}

    {#snippet rowActions(p: LlmProvider)}
      <button
        type="button"
        data-testid="test-provider-{p.id}"
        onclick={() => void runProviderTest(p)}
        disabled={providerTests[p.id]?.running}
        class="rounded px-3 py-1 text-xs font-semibold text-brand-blue
               transition-colors hover:bg-brand-blue/10 disabled:opacity-40"
      >
        {providerTests[p.id]?.running ? 'Testing…' : 'Test'}
      </button>
      <button
        type="button"
        onclick={() => startEdit(p)}
        class="rounded px-3 py-1 text-xs font-semibold text-brand-blue
               transition-colors hover:bg-brand-blue/10"
      >
        Edit
      </button>
      <button
        type="button"
        onclick={() => onDeleteProvider(p)}
        disabled={deletingId === p.id}
        class="rounded px-3 py-1 text-xs font-semibold text-red-500
               transition-colors hover:bg-red-50 disabled:opacity-40"
      >
        {deletingId === p.id ? 'Deleting…' : 'Delete'}
      </button>
    {/snippet}

    {#snippet testResult(p: LlmProvider)}
      {@const t = providerTests[p.id]}
      {#if t}
        <div
          class="rounded-md border px-3 py-2 text-xs
                 {t.running
            ? 'border-brand-border/60 bg-brand-light-blue/20'
            : t.ok
              ? 'border-green-200 bg-green-50'
              : 'border-red-200 bg-red-50'}"
          data-testid="test-result-{p.id}"
        >
          <div class="flex flex-wrap items-center gap-x-3 gap-y-1.5">
            {#if t.running}
              <span class="font-semibold text-brand-muted">Calling the provider…</span>
            {:else if t.ok}
              <span class="font-semibold text-green-700">Works</span>
            {:else}
              <span class="font-semibold text-red-700">Failed</span>
            {/if}
            <!-- Fixed only from `sm` up: a hard w-56 is wider than the mobile
                 card leaves and pushed the row into horizontal scroll. -->
            <label class="flex min-w-0 flex-1 items-center gap-1.5 text-brand-muted sm:flex-none">
              model
              <input
                type="text"
                list="test-models-{p.id}"
                value={t.model}
                oninput={(e) => setTestModel(p, e.currentTarget.value)}
                onfocus={() => ensureTestModelOptions(p)}
                placeholder="pick or type a model id"
                class="min-w-0 flex-1 rounded border border-brand-border bg-white px-2 py-0.5 font-mono
                       text-[11px] text-brand-text focus:outline-none focus:ring-1 focus:ring-brand-blue
                       sm:w-56 sm:flex-none"
              />
              <datalist id="test-models-{p.id}">
                {#each testModelOptions[p.id] ?? [] as m (m)}
                  <option value={m}></option>
                {/each}
              </datalist>
            </label>
            {#if t.latency_ms != null}
              <span class="tabular-nums text-brand-muted">{(t.latency_ms / 1000).toFixed(1)}s</span>
            {/if}
            <span class="ml-auto flex items-center gap-1">
              <button
                type="button"
                onclick={() => void runProviderTest(p)}
                disabled={t.running}
                class="rounded px-2 py-0.5 font-semibold text-brand-blue transition-colors
                       hover:bg-brand-blue/10 disabled:opacity-40"
              >
                Run again
              </button>
              <button
                type="button"
                onclick={() => dismissProviderTest(p)}
                class="rounded px-2 py-0.5 font-semibold text-brand-muted transition-colors
                       hover:bg-brand-light-blue/30"
                aria-label="Dismiss test result"
              >
                ✕
              </button>
            </span>
          </div>
          {#if !t.running && t.ok && t.output}
            <p class="mt-1.5 break-words font-mono text-[11px] text-green-800">↳ {t.output}</p>
          {/if}
          {#if !t.running && t.error}
            <!-- The provider's own words: "Insufficient balance" beats "502". -->
            <p class="mt-1.5 break-words font-mono text-[11px] text-red-700">{t.error}</p>
          {/if}
        </div>
      {/if}
    {/snippet}

    {#snippet baseUrlCell(p: LlmProvider, widthClass: string)}
      {#if p.base_url}
        <!-- Long gateway URLs (e.g. Cloudflare AI Gateway) must not stretch the
             table: capped width + ellipsis, full URL on hover and via copy. -->
        <span class="flex items-center gap-1.5 {widthClass}">
          <span class="min-w-0 truncate font-mono text-xs" title={p.base_url}>
            {p.base_url.replace(/^https?:\/\//, '')}
          </span>
          <button
            type="button"
            class="shrink-0 rounded p-0.5 text-brand-muted/60 transition-colors hover:bg-brand-light-blue/20 hover:text-brand-text"
            title="Copy base URL"
            aria-label="Copy base URL"
            onclick={() => navigator.clipboard?.writeText(p.base_url ?? '')}
          >
            <svg class="h-3.5 w-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5">
              <rect x="5.5" y="5.5" width="8" height="8" rx="1.5" />
              <path d="M10.5 5.5v-2a1.5 1.5 0 0 0-1.5-1.5H4A1.5 1.5 0 0 0 2.5 3.5V9A1.5 1.5 0 0 0 4 10.5h1.5" />
            </svg>
          </button>
        </span>
      {:else}
        <span class="text-brand-muted/60">—</span>
      {/if}
    {/snippet}

    <div class="rounded-[8px] bg-white shadow-sm">
      {#if providers.length === 0}
        <div class="flex flex-col items-center justify-center py-16 text-brand-muted">
          <p class="text-sm">No providers yet. Create one above.</p>
        </div>
      {:else}
        <ul class="divide-y divide-brand-border/60 xl:hidden">
          {#each providers as p (p.id)}
            <li class="flex flex-col gap-2.5 p-4">
              <div class="flex items-start justify-between gap-3">
                {@render providerIdentity(p)}
                {@render enabledToggle(p)}
              </div>

              <dl class="grid grid-cols-[4.5rem_minmax(0,1fr)] items-center gap-x-3 gap-y-1.5 text-xs text-brand-muted">
                <dt class="font-semibold uppercase tracking-wider text-brand-muted/70">Base URL</dt>
                <dd class="min-w-0">{@render baseUrlCell(p, 'w-full')}</dd>
                <dt class="font-semibold uppercase tracking-wider text-brand-muted/70">API key</dt>
                <dd class="min-w-0">{@render keyState(p)}</dd>
              </dl>

              <div class="flex justify-end gap-2">{@render rowActions(p)}</div>
              {@render testResult(p)}
            </li>
          {/each}
        </ul>

        <!-- Every cell is whitespace-nowrap so auto table layout can never
             squeeze a column below its content. -->
        <div class="hidden overflow-x-auto xl:block">
          <table class="w-full">
            <thead>
              <tr class="border-b border-brand-border bg-brand-light-blue/40 text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
                <th class="whitespace-nowrap py-3 pl-6 pr-4 text-left">Provider</th>
                <th class="whitespace-nowrap px-4 py-3 text-left">Base URL</th>
                <th class="whitespace-nowrap px-4 py-3 text-left">API key</th>
                <th class="whitespace-nowrap px-4 py-3 text-left">Status</th>
                <th class="whitespace-nowrap py-3 pl-4 pr-6 text-right">Actions</th>
              </tr>
            </thead>
            <tbody>
              {#each providers as p (p.id)}
                <tr class="border-b border-brand-border/60 last:border-0 hover:bg-brand-light-blue/20">
                  <td class="whitespace-nowrap py-3 pl-6 pr-4">{@render providerIdentity(p)}</td>
                  <td class="px-4 py-3 text-sm text-brand-muted">
                    {@render baseUrlCell(p, 'max-w-[260px]')}
                  </td>
                  <td class="whitespace-nowrap px-4 py-3 text-sm">{@render keyState(p)}</td>
                  <td class="whitespace-nowrap px-4 py-3">{@render enabledToggle(p)}</td>
                  <td class="whitespace-nowrap py-3 pl-4 pr-6 text-right">
                    <div class="flex justify-end gap-2">{@render rowActions(p)}</div>
                  </td>
                </tr>
                {#if providerTests[p.id]}
                  <tr class="border-b border-brand-border/60 last:border-0">
                    <td colspan="5" class="px-6 py-2">{@render testResult(p)}</td>
                  </tr>
                {/if}
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </div>
  </section>

  <!-- ============ Model pools ============ -->
  <section>
    <div class="mb-4 flex items-start justify-between gap-4">
      <div>
        <h2 class="font-heading text-[20px] font-semibold text-brand-text">Model pools</h2>
        <p class="mt-0.5 text-sm text-brand-muted">
          A pool is a list of models tried in order. Members sharing a priority form one tier and
          split the load between them; the next tier is only reached once the current one fails.
        </p>
      </div>
      {#if poolEditor === null}
        <button
          type="button"
          onclick={startCreatePool}
          class="shrink-0 rounded-btn bg-brand-blue px-5 py-2 text-sm font-semibold text-white
                 transition-opacity hover:opacity-80"
        >
          New pool
        </button>
      {/if}
    </div>

    {#if poolEditor !== null}
      <div class="mb-6 rounded-[8px] bg-white p-6 shadow-sm">
        <div class="grid gap-4 sm:grid-cols-2">
          <div>
            <label for="pool-name" class="mb-1 block text-xs font-semibold text-brand-text">
              Name
            </label>
            <input
              id="pool-name"
              type="text"
              value={poolName}
              oninput={(e) => (poolName = e.currentTarget.value)}
              placeholder="Fast tier"
              class="w-full rounded-[6px] border border-brand-border px-3 py-2 text-sm text-brand-text
                     focus:outline-none focus:ring-2 focus:ring-brand-blue"
            />
          </div>
          <div>
            <label for="pool-desc" class="mb-1 block text-xs font-semibold text-brand-text">
              Description
            </label>
            <input
              id="pool-desc"
              type="text"
              value={poolDescription}
              oninput={(e) => (poolDescription = e.currentTarget.value)}
              placeholder="Optional"
              class="w-full rounded-[6px] border border-brand-border px-3 py-2 text-sm text-brand-text
                     focus:outline-none focus:ring-2 focus:ring-brand-blue"
            />
          </div>
        </div>

        <div class="mt-5">
          <div class="mb-2 flex items-center justify-between">
            <div class="text-xs font-semibold text-brand-text">Members</div>
            <button
              type="button"
              onclick={addMember}
              class="rounded px-3 py-1 text-xs font-semibold text-brand-blue
                     transition-colors hover:bg-brand-blue/10"
            >
              + Add member
            </button>
          </div>

          {#if poolMembers.length === 0}
            <p class="rounded-[6px] border border-dashed border-brand-border px-3 py-6 text-center text-xs text-brand-muted">
              No members yet. A pool with no members is ignored at resolve time.
            </p>
          {:else}
            <!-- Column headers: "Tier" is not self-evident from a bare number
                 input, and the load-splitting rule lives here rather than in a
                 tooltip nobody hovers. Below `sm` the selector column is gone
                 (each selector wraps onto its own line), so its heading spans
                 the row and the small-field headings stay over their column. -->
            <div
              class="mb-1.5 grid grid-cols-[5rem_3.5rem_minmax(0,5rem)] justify-start items-end gap-3
                     text-[11px] font-semibold uppercase tracking-wider text-brand-muted
                     sm:grid-cols-[minmax(0,28rem)_5rem_3.5rem_5rem]"
            >
              <div class="col-span-3 sm:col-span-1">Provider &amp; model</div>
              <div>Tier</div>
              <div>Enabled</div>
              <div></div>
            </div>
            <div class="flex flex-col gap-2">
              {#each poolMembers as m, i (i)}
                <!-- One 28rem column plus three small ones does not fit a
                     phone: below `sm` the selector takes the whole first line
                     and the tier/enabled/remove trio shares the second. -->
                <div
                  class="grid grid-cols-[5rem_3.5rem_minmax(0,5rem)] justify-start items-center gap-3
                         sm:grid-cols-[minmax(0,28rem)_5rem_3.5rem_5rem]"
                >
                  <div class="col-span-3 min-w-0 sm:col-span-1">
                    <ModelSelector
                      {providers}
                      modelsFor={suggestionsFor}
                      value={m.provider_id && m.model
                        ? { provider_id: m.provider_id, model: m.model }
                        : null}
                      onchange={(sel) =>
                        patchMember(i, {
                          provider_id: sel?.provider_id ?? '',
                          model: sel?.model ?? '',
                        })}
                      placeholder="Pick a provider and model…"
                      label="Member {i + 1} model"
                      allowClear={!!m.provider_id}
                      clearLabel="Clear this member"
                    />
                  </div>
                  <input
                    type="number"
                    aria-label="Member {i + 1} tier"
                    title="Lower is tried first; members sharing a tier split the load"
                    value={m.priority}
                    oninput={(e) => patchMember(i, { priority: e.currentTarget.value })}
                    class="w-full rounded-[6px] border border-brand-border px-2 py-2 text-sm text-brand-text
                           focus:outline-none focus:ring-2 focus:ring-brand-blue"
                  />
                  <div class="flex justify-center">
                    <input
                      type="checkbox"
                      aria-label="Member {i + 1} enabled"
                      checked={m.enabled}
                      onchange={(e) => patchMember(i, { enabled: e.currentTarget.checked })}
                      class="h-4 w-4"
                    />
                  </div>
                  <button
                    type="button"
                    aria-label="Remove member {i + 1}"
                    onclick={() => removeMember(i)}
                    class="rounded px-2 py-1 text-xs font-semibold text-red-500
                           transition-colors hover:bg-red-50"
                  >
                    Remove
                  </button>
                </div>
              {/each}
            </div>
          {/if}
        </div>

        {#if poolPreview.length > 0}
          <div class="mt-4 rounded-[6px] bg-brand-light-blue/40 px-3 py-2">
            <div class="text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
              Resolution order
            </div>
            <ol class="mt-1 flex flex-col gap-0.5">
              {#each poolPreview as [priority, labels] (priority)}
                <li class="text-xs text-brand-text">
                  <span class="font-semibold">Tier {priority}</span>
                  — {labels.join(', ')}
                  {#if labels.length > 1}<span class="text-brand-muted"> (load split)</span>{/if}
                </li>
              {/each}
            </ol>
          </div>
        {/if}

        {#if poolError}
          <p class="mt-3 text-xs text-red-500">{poolError}</p>
        {/if}

        <div class="mt-5 flex gap-3">
          <button
            type="button"
            onclick={savePool}
            disabled={poolSaving}
            class="rounded-btn bg-brand-blue px-5 py-2 text-sm font-semibold text-white
                   transition-opacity hover:opacity-80 disabled:opacity-40"
          >
            {poolSaving ? 'Saving…' : poolEditor === 'create' ? 'Create pool' : 'Save pool'}
          </button>
          <button
            type="button"
            onclick={cancelPoolEditor}
            class="rounded-btn border border-brand-border px-5 py-2 text-sm font-semibold text-brand-text
                   transition-colors hover:bg-brand-light-blue/40"
          >
            Cancel
          </button>
        </div>
      </div>
    {/if}

    <!-- Same split as the providers list: cards below `xl`, table above it,
         cell markup shared via snippets so the two layouts cannot drift. -->
    {#snippet poolIdentity(p: LlmPool)}
      <div class="min-w-0">
        <p class="text-sm font-medium text-brand-text">{p.name}</p>
        {#if p.description}
          <p class="mt-0.5 text-xs text-brand-muted">{p.description}</p>
        {/if}
      </div>
    {/snippet}

    {#snippet poolMembersCell(p: LlmPool)}
      <div class="min-w-0">
        <p class="text-sm text-brand-text">{tierSummary(p)}</p>
        <p class="mt-0.5 break-words text-xs text-brand-muted">
          {p.members
            .filter((m) => m.enabled)
            .map((m) => m.model)
            .join(', ') || '—'}
        </p>
      </div>
    {/snippet}

    {#snippet poolActions(p: LlmPool)}
      <button
        type="button"
        onclick={() => startEditPool(p)}
        class="rounded px-3 py-1 text-xs font-semibold text-brand-blue
               transition-colors hover:bg-brand-blue/10"
      >
        Edit
      </button>
      <button
        type="button"
        onclick={() => onDeletePool(p)}
        disabled={poolDeletingId === p.id}
        class="rounded px-3 py-1 text-xs font-semibold text-red-500
               transition-colors hover:bg-red-50 disabled:opacity-40"
      >
        {poolDeletingId === p.id ? 'Deleting…' : 'Delete'}
      </button>
    {/snippet}

    <div class="overflow-hidden rounded-[8px] bg-white shadow-sm">
      {#if pools.length === 0}
        <div class="flex flex-col items-center justify-center py-16 text-brand-muted">
          <p class="text-sm">No pools yet.</p>
          <p class="mt-1 text-xs">
            Create one to give an operation or a judge several models to fall back through.
          </p>
        </div>
      {:else}
        <ul class="divide-y divide-brand-border/60 xl:hidden">
          {#each pools as p (p.id)}
            <li class="flex flex-col gap-2.5 p-4">
              {@render poolIdentity(p)}
              {@render poolMembersCell(p)}
              <div class="flex justify-end gap-2">{@render poolActions(p)}</div>
            </li>
          {/each}
        </ul>

        <table class="hidden w-full xl:table">
          <thead>
            <tr class="border-b border-brand-border bg-brand-light-blue/40">
              <th class="px-6 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
                Pool
              </th>
              <th class="px-6 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
                Members
              </th>
              <th class="px-6 py-3 text-right text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
                Actions
              </th>
            </tr>
          </thead>
          <tbody>
            {#each pools as p (p.id)}
              <tr class="border-b border-brand-border/60 last:border-0 hover:bg-brand-light-blue/20">
                <td class="px-6 py-4">{@render poolIdentity(p)}</td>
                <td class="px-6 py-4">{@render poolMembersCell(p)}</td>
                <td class="px-6 py-4 text-right">
                  <div class="flex justify-end gap-2">{@render poolActions(p)}</div>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </div>
  </section>

  <!-- ============ Default model ============ -->
  <section>
    <div class="mb-4">
      <h2 class="font-heading text-[20px] font-semibold text-brand-text">Default model</h2>
      <p class="mt-0.5 text-sm text-brand-muted">
        Used for every AI operation without a dedicated assignment below.
      </p>
    </div>
    <div class="rounded-[8px] bg-white p-6 shadow-sm">
      <!-- items-end, not items-center: the selector is two rows tall (the
           Model/Pool switch sits above the control), so centring floats the
           button halfway up instead of sitting it on the control's line. -->
      <div class="flex flex-wrap items-end gap-3">
        <div class="min-w-0 flex-1 sm:max-w-md">
          <AssignmentSelector
            {providers}
            {pools}
            value={defaultForm}
            onchange={onDefaultChange}
            modelsFor={suggestionsFor}
            placeholder="Select default model…"
            label="Default model"
            clearLabel="Clear default model"
          />
        </div>
        <button
          type="button"
          onclick={saveDefault}
          disabled={busyKey !== null
            || !assignmentComplete(defaultForm)
            || !assignmentDirty(defaultForm, assignments.default)}
          class="h-[38px] shrink-0 rounded-btn bg-brand-blue px-5 text-sm font-semibold text-white
                 transition-opacity hover:opacity-80 disabled:opacity-40"
        >
          {busyKey === 'default' ? 'Saving…' : 'Save'}
        </button>
        <!-- Clearing the default leaves every operation without one, so this
             one is worded as a removal and confirmed before it fires. -->
        {#if assignments.default !== null}
          <button
            type="button"
            onclick={confirmClearDefault}
            disabled={busyKey !== null}
            title="Remove the default model — operations without their own assignment stop working until one is set"
            class="h-[38px] shrink-0 rounded-btn border border-brand-border px-5 text-sm font-semibold
                   text-brand-text transition-colors hover:bg-brand-light-blue/40 disabled:opacity-40"
          >
            Clear
          </button>
        {/if}
      </div>
      {#if assignments.default === null}
        <p class="mt-2 text-xs text-brand-muted">No default model configured.</p>
      {/if}
    </div>
  </section>

  <!-- ============ Operations ============ -->
  <section>
    <div class="mb-4">
      <h2 class="font-heading text-[20px] font-semibold text-brand-text">Operations</h2>
      <p class="mt-0.5 text-sm text-brand-muted">
        Assign a dedicated provider and model per operation, or inherit the default.
      </p>
    </div>
    <!-- No overflow-x-auto here: it would clip the ModelSelector dropdown.
         That rules out the scroll-to-fit escape hatch on a phone too, so the
         narrow layout is stacked cards (label, then a full-width selector,
         then the actions) instead — the table only renders from `xl` up. -->
    {#snippet opLabel(op: { key: LlmOperation; label: string }, stored: LlmAssignment | null)}
      <div class="text-sm font-medium text-brand-text">
        {op.label}
        {#if stored === null}
          <p class="mt-0.5 text-xs text-brand-muted">Inherits default</p>
        {/if}
      </div>
    {/snippet}

    {#snippet opSelector(op: { key: LlmOperation; label: string })}
      <AssignmentSelector
        {providers}
        {pools}
        value={opForms[op.key]}
        onchange={(sel) => onOpChange(op.key, sel)}
        modelsFor={suggestionsFor}
        placeholder="Inherits default model…"
        label="{op.label} model"
        clearLabel="Inherit default"
      />
    {/snippet}

    {#snippet opActions(op: { key: LlmOperation; label: string }, stored: LlmAssignment | null)}
      <button
        type="button"
        onclick={() => saveOperation(op.key)}
        disabled={busyKey !== null
          || !assignmentComplete(opForms[op.key])
          || !assignmentDirty(opForms[op.key], stored)}
        class="rounded px-3 py-1 text-xs font-semibold text-brand-blue
               transition-colors hover:bg-brand-blue/10 disabled:opacity-40"
      >
        {busyKey === `op:${op.key}` ? 'Saving…' : 'Save'}
      </button>
      <!-- Only meaningful once something is stored; clearing is a
           revert to the default assignment, not a deletion, so it
           is not styled as destructive. -->
      {#if stored !== null}
        <button
          type="button"
          onclick={() => clearOperation(op.key)}
          disabled={busyKey !== null}
          title="Remove this assignment — the operation falls back to the default model"
          class="rounded px-3 py-1 text-xs font-semibold text-brand-muted
                 transition-colors hover:bg-brand-light-blue/40 hover:text-brand-text
                 disabled:opacity-40"
        >
          Clear
        </button>
      {/if}
    {/snippet}

    <div class="rounded-[8px] bg-white shadow-sm">
      <ul class="divide-y divide-brand-border/60 xl:hidden">
        {#each OPERATIONS as op (op.key)}
          {@const stored = assignments.operations[op.key] ?? null}
          <li class="flex flex-col gap-2.5 p-4">
            {@render opLabel(op, stored)}
            {@render opSelector(op)}
            <div class="flex justify-end gap-2">{@render opActions(op, stored)}</div>
          </li>
        {/each}
      </ul>

      <table class="hidden w-full xl:table">
        <thead>
          <tr class="border-b border-brand-border bg-brand-light-blue/40">
            <th class="px-6 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Operation</th>
            <th class="px-6 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Provider / model</th>
            <th class="px-6 py-3 text-right text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Actions</th>
          </tr>
        </thead>
        <tbody>
          {#each OPERATIONS as op (op.key)}
            {@const stored = assignments.operations[op.key] ?? null}
            <tr class="border-b border-brand-border/60 last:border-0 hover:bg-brand-light-blue/20">
              <td class="px-6 py-4">{@render opLabel(op, stored)}</td>
              <td class="px-6 py-4">{@render opSelector(op)}</td>
              <!-- align-bottom keeps the action on the selector's control
                   line; the default middle alignment floats it against the
                   Model/Pool switch instead. -->
              <td class="px-6 py-4 text-right align-bottom">
                <div class="flex justify-end gap-2">{@render opActions(op, stored)}</div>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  </section>

  <!-- ============ Per-judge models ============ -->
  <section>
    <div class="mb-4">
      <h2 class="font-heading text-[20px] font-semibold text-brand-text">Per-judge models</h2>
      <p class="mt-0.5 text-sm text-brand-muted">
        Override the judge model per judge; unset judges inherit the "Judges (default)" assignment.
      </p>
    </div>
    <!-- No overflow-x-auto here: it would clip the ModelSelector dropdown.
         Stacked cards below `xl` for the same reason as the Operations
         section — a phone gets no scroll container to hide behind. -->
    {#snippet judgeLabel(j: Judge)}
      <div class="min-w-0">
        <p class="text-sm font-medium text-brand-text">{j.name}</p>
        <p class="mt-0.5 text-xs text-brand-muted">
          {j.slug}{judgeHasOverride(j) ? '' : ' — inherits default'}
        </p>
      </div>
    {/snippet}

    {#snippet judgeSelector(j: Judge)}
      {#if judgeHasBoth(j)}
        <!-- This table shows one source per judge; a judge using
             a pool AND a pinned model also needs an order, which
             only the judges page can set. -->
        <p class="text-sm text-brand-text">Pool + model</p>
        <a
          href="/settings/judges"
          class="text-xs font-semibold text-brand-blue hover:underline"
        >
          Edit on the judges page →
        </a>
      {:else}
        <AssignmentSelector
          {providers}
          {pools}
          value={judgeForms[j.id] ?? null}
          onchange={(sel) => onJudgeChange(j, sel)}
          modelsFor={suggestionsFor}
          placeholder="Inherits judge default…"
          label="{j.name} model"
          clearLabel="Inherit default"
        />
      {/if}
    {/snippet}

    {#snippet judgeActions(j: Judge)}
      <button
        type="button"
        onclick={() => saveJudgeOverride(j)}
        disabled={busyKey !== null
          || judgeHasBoth(j)
          || !assignmentComplete(judgeForms[j.id] ?? null)
          || !assignmentDirty(judgeForms[j.id] ?? null, judgeStored(j))}
        class="rounded px-3 py-1 text-xs font-semibold text-brand-blue
               transition-colors hover:bg-brand-blue/10 disabled:opacity-40"
      >
        {busyKey === `judge:${j.id}` ? 'Saving…' : 'Save'}
      </button>
      <!-- Shown for any override, including the pool+model pair
           this table cannot otherwise edit — removing it from
           here is still unambiguous. -->
      {#if judgeHasOverride(j)}
        <button
          type="button"
          onclick={() => clearJudgeOverride(j)}
          disabled={busyKey !== null}
          title="Remove this override — the judge falls back to the Judges (default) assignment"
          class="rounded px-3 py-1 text-xs font-semibold text-brand-muted
                 transition-colors hover:bg-brand-light-blue/40 hover:text-brand-text
                 disabled:opacity-40"
        >
          Clear
        </button>
      {/if}
    {/snippet}

    <div class="rounded-[8px] bg-white shadow-sm">
      {#if judges.length === 0}
        <div class="flex flex-col items-center justify-center py-16 text-brand-muted">
          <p class="text-sm">No judges defined. Create them in the Judges tab.</p>
        </div>
      {:else}
        <ul class="divide-y divide-brand-border/60 xl:hidden">
          {#each judges as j (j.id)}
            <li class="flex flex-col gap-2.5 p-4">
              {@render judgeLabel(j)}
              <div>{@render judgeSelector(j)}</div>
              <div class="flex justify-end gap-2">{@render judgeActions(j)}</div>
            </li>
          {/each}
        </ul>

        <table class="hidden w-full xl:table">
          <thead>
            <tr class="border-b border-brand-border bg-brand-light-blue/40">
              <th class="px-6 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Judge</th>
              <th class="px-6 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Provider / model</th>
              <th class="px-6 py-3 text-right text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Actions</th>
            </tr>
          </thead>
          <tbody>
            {#each judges as j (j.id)}
              <tr class="border-b border-brand-border/60 last:border-0 hover:bg-brand-light-blue/20">
                <td class="px-6 py-4">{@render judgeLabel(j)}</td>
                <td class="px-6 py-4">{@render judgeSelector(j)}</td>
                <td class="px-6 py-4 text-right align-bottom">
                  <div class="flex justify-end gap-2">{@render judgeActions(j)}</div>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </div>
  </section>

</div>
