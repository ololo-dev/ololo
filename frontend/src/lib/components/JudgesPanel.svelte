<script lang="ts">
  import { invalidateAll } from "$app/navigation";
  import {
    attachTaskJudge,
    updateTaskJudge,
    detachTaskJudge,
    ApiError,
    type TaskJudge,
    type Judge,
    type RatingScale,
    type Project,
  } from "$lib/api";

  interface Props {
    project: Project;
    selectedTaskId: string | null;
    judges?: Judge[];
    taskJudgesMap: Record<string, TaskJudge[]>;
  }

  let { project, selectedTaskId, judges, taskJudgesMap }: Props = $props();

  let drawerTaskJudges = $state<TaskJudge[]>([]);
  let judgeAttachError = $state<string | null>(null);
  let judgeAttaching = $state(false);
  let judgeDetachingId = $state<string | null>(null);
  let judgeUpdatingId = $state<string | null>(null);
  let judgeUpdateError = $state<string | null>(null);
  let selectedJudgeId = $state("");
  let judgeOrdinalInput = $state("1");
  let judgeUseOverride = $state(false);
  let judgeOvMin = $state("0");
  let judgeOvMax = $state("10");
  let judgeOvStep = $state("0.5");
  let editJudgeOrdinal = $state<Record<string, string>>({});
  let editJudgeUseOverride = $state<Record<string, boolean>>({});
  let editJudgeMin = $state<Record<string, string>>({});
  let editJudgeMax = $state<Record<string, string>>({});
  let editJudgeStep = $state<Record<string, string>>({});

  function syncJudgeEditState(tjs: TaskJudge[]) {
    const ord: Record<string, string> = {};
    const ov: Record<string, boolean> = {};
    const mn: Record<string, string> = {};
    const mx: Record<string, string> = {};
    const st: Record<string, string> = {};
    for (const tj of tjs) {
      ord[tj.id] = String(tj.ordinal);
      ov[tj.id] = tj.rating_scale_override !== null;
      mn[tj.id] = String(tj.rating_scale_override?.min ?? tj.effective_rating_scale.min);
      mx[tj.id] = String(tj.rating_scale_override?.max ?? tj.effective_rating_scale.max);
      st[tj.id] = String(tj.rating_scale_override?.step ?? tj.effective_rating_scale.step);
    }
    editJudgeOrdinal = ord;
    editJudgeUseOverride = ov;
    editJudgeMin = mn;
    editJudgeMax = mx;
    editJudgeStep = st;
  }

  function scaleSummary(rs: RatingScale): string {
    return `${rs.min}–${rs.max} step ${rs.step}`;
  }

  function parseJudgeOverride(minStr: string, maxStr: string, stepStr: string): RatingScale | null {
    const min = Number(minStr);
    const max = Number(maxStr);
    const step = Number(stepStr);
    if (!Number.isFinite(min) || !Number.isFinite(max) || !Number.isFinite(step)) return null;
    if (step <= 0) return null;
    if (max <= min) return null;
    return { min, max, step };
  }

  const attachedJudgeIds = $derived(new Set(drawerTaskJudges.map((tj) => tj.judge_id)));
  const unattachedJudges = $derived(
    (judges as Judge[] | undefined)?.filter((j) => !attachedJudgeIds.has(j.id)) ?? [],
  );

  $effect(() => {
    const tid = selectedTaskId;
    const tjs = (tid && tid !== "new" ? taskJudgesMap?.[tid] : undefined) ?? [];
    drawerTaskJudges = tjs;
    syncJudgeEditState(tjs);
    judgeAttachError = null;
    judgeUpdateError = null;
  });

  async function onJudgeAttach() {
    judgeAttachError = null;
    const pid = project.id;
    const tid = selectedTaskId;
    if (!tid || tid === "new") return;
    const judgeId = selectedJudgeId;
    const ordinal = Number(judgeOrdinalInput);
    if (!judgeId) { judgeAttachError = "Select a judge."; return; }
    if (!Number.isFinite(ordinal) || ordinal < 0) { judgeAttachError = "Invalid ordinal."; return; }
    let override: RatingScale | undefined;
    if (judgeUseOverride) {
      const ov = parseJudgeOverride(judgeOvMin, judgeOvMax, judgeOvStep);
      if (!ov) { judgeAttachError = "Invalid override scale: max must exceed min, step positive."; return; }
      override = ov;
    }
    judgeAttaching = true;
    try {
      await attachTaskJudge(pid, tid, {
        judge_id: judgeId,
        ordinal,
        ...(override ? { rating_scale_override: override } : {}),
      });
      selectedJudgeId = "";
      judgeOrdinalInput = "1";
      judgeUseOverride = false;
      judgeOvMin = "0";
      judgeOvMax = "10";
      judgeOvStep = "0.5";
      await invalidateAll();
      drawerTaskJudges = taskJudgesMap[tid] ?? [];
    } catch (err) {
      if (err instanceof ApiError) {
        if (err.status === 409) judgeAttachError = "Judge or ordinal already in use.";
        else if (err.status === 400) judgeAttachError = "Invalid request.";
        else judgeAttachError = err.code ?? `Error ${err.status}`;
      } else {
        judgeAttachError = "Unknown error.";
      }
    } finally {
      judgeAttaching = false;
    }
  }

  async function onJudgeDetach(tj: TaskJudge) {
    const pid = project.id;
    const tid = selectedTaskId;
    if (!tid || tid === "new") return;
    if (!confirm(`Detach judge "${tj.judge_name}" from this task?`)) return;
    judgeDetachingId = tj.id;
    try {
      await detachTaskJudge(pid, tid, tj.judge_id);
      await invalidateAll();
      drawerTaskJudges = taskJudgesMap[tid] ?? [];
    } catch (err) {
      if (err instanceof ApiError) {
        judgeAttachError = err.code ?? `Error ${err.status}`;
      } else {
        judgeAttachError = "Could not detach judge.";
      }
    } finally {
      judgeDetachingId = null;
    }
  }

  async function onJudgeUpdate(tj: TaskJudge) {
    judgeUpdateError = null;
    const pid = project.id;
    const tid = selectedTaskId;
    if (!tid || tid === "new") return;
    const ordinal = Number(editJudgeOrdinal[tj.id] ?? String(tj.ordinal));
    if (!Number.isFinite(ordinal)) { judgeUpdateError = "Invalid ordinal."; return; }
    const body: { ordinal?: number; rating_scale_override?: RatingScale | null } = {};
    if (ordinal !== tj.ordinal) body.ordinal = ordinal;
    if (editJudgeUseOverride[tj.id]) {
      const ov = parseJudgeOverride(editJudgeMin[tj.id], editJudgeMax[tj.id], editJudgeStep[tj.id]);
      if (!ov) { judgeUpdateError = "Invalid override scale."; return; }
      const current = tj.rating_scale_override;
      const changed = !current || current.min !== ov.min || current.max !== ov.max || current.step !== ov.step;
      if (changed) body.rating_scale_override = ov;
    } else if (tj.rating_scale_override !== null) {
      body.rating_scale_override = null;
    }
    if (Object.keys(body).length === 0) return;
    judgeUpdatingId = tj.id;
    try {
      await updateTaskJudge(pid, tid, tj.judge_id, body);
      await invalidateAll();
      drawerTaskJudges = taskJudgesMap[tid] ?? [];
    } catch (err) {
      if (err instanceof ApiError) {
        if (err.status === 409) judgeUpdateError = "Ordinal already in use.";
        else if (err.status === 400) judgeUpdateError = "Invalid request.";
        else judgeUpdateError = err.code ?? `Error ${err.status}`;
      } else {
        judgeUpdateError = "Unknown error.";
      }
    } finally {
      judgeUpdatingId = null;
    }
  }
</script>

{#if judges && judges.length > 0 || drawerTaskJudges.length > 0}
  <!-- Judges (admin only) -->
  <div class="mt-5 border-t border-brand-border pt-4">
    <h3 class="mb-3 font-heading text-base font-semibold text-brand-text">
      Judges <span class="text-xs font-normal text-brand-muted">(AI evaluators attached to this task)</span>
    </h3>

    {#if drawerTaskJudges.length > 0}
      <div class="mb-4 overflow-hidden rounded-[8px] border border-brand-border">
        <table class="w-full">
          <thead>
            <tr class="border-b border-brand-border bg-brand-light-blue/40">
              <th class="px-3 py-2 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Judge</th>
              <th class="px-3 py-2 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Ordinal</th>
              <th class="px-3 py-2 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Scale</th>
              <th class="px-3 py-2 text-right text-[11px] font-semibold uppercase tracking-wider text-brand-muted">Actions</th>
            </tr>
          </thead>
          <tbody>
            {#each drawerTaskJudges as tj (tj.id)}
              <tr class="border-b border-brand-border/60 last:border-0 align-top">
                <td class="px-3 py-3 text-sm font-medium text-brand-text">
                  {tj.judge_name}
                  <div class="mt-0.5 text-xs text-brand-muted">{tj.judge_slug}</div>
                </td>
                <td class="px-3 py-3 text-sm text-brand-text">
                  <input
                    type="number"
                    min="0"
                    value={editJudgeOrdinal[tj.id] ?? String(tj.ordinal)}
                    oninput={(e) => { editJudgeOrdinal = { ...editJudgeOrdinal, [tj.id]: (e.target as HTMLInputElement).value }; }}
                    class="h-[32px] w-[64px] rounded-[6px] border border-brand-border px-2 text-xs text-brand-text focus:border-brand-blue focus:outline-none"
                  />
                </td>
                <td class="px-3 py-3 text-xs text-brand-muted">
                  <div>{scaleSummary(tj.effective_rating_scale)}</div>
                  <label class="mt-1 flex items-center gap-1">
                    <input
                      type="checkbox"
                      checked={editJudgeUseOverride[tj.id] ?? false}
                      onchange={(e) => { editJudgeUseOverride = { ...editJudgeUseOverride, [tj.id]: (e.target as HTMLInputElement).checked }; }}
                    />
                    <span>override</span>
                  </label>
                  {#if editJudgeUseOverride[tj.id]}
                    <div class="mt-1 grid grid-cols-3 gap-1">
                      <input type="number" step="0.1" placeholder="min" value={editJudgeMin[tj.id] ?? ''}
                        oninput={(e) => { editJudgeMin = { ...editJudgeMin, [tj.id]: (e.target as HTMLInputElement).value }; }}
                        class="h-[28px] rounded-[4px] border border-brand-border px-1 text-xs text-brand-text focus:border-brand-blue focus:outline-none" />
                      <input type="number" step="0.1" placeholder="max" value={editJudgeMax[tj.id] ?? ''}
                        oninput={(e) => { editJudgeMax = { ...editJudgeMax, [tj.id]: (e.target as HTMLInputElement).value }; }}
                        class="h-[28px] rounded-[4px] border border-brand-border px-1 text-xs text-brand-text focus:border-brand-blue focus:outline-none" />
                      <input type="number" step="0.1" placeholder="step" value={editJudgeStep[tj.id] ?? ''}
                        oninput={(e) => { editJudgeStep = { ...editJudgeStep, [tj.id]: (e.target as HTMLInputElement).value }; }}
                        class="h-[28px] rounded-[4px] border border-brand-border px-1 text-xs text-brand-text focus:border-brand-blue focus:outline-none" />
                    </div>
                  {/if}
                </td>
                <td class="px-3 py-3 text-right">
                  <div class="flex justify-end gap-1">
                    <button type="button" onclick={() => onJudgeUpdate(tj)} disabled={judgeUpdatingId === tj.id}
                      class="rounded px-2 py-0.5 text-xs font-semibold text-brand-blue transition-colors hover:bg-brand-blue/10 disabled:opacity-40">
                      {judgeUpdatingId === tj.id ? "Saving…" : "Save"}
                    </button>
                    <button type="button" onclick={() => onJudgeDetach(tj)} disabled={judgeDetachingId === tj.id}
                      class="rounded px-2 py-0.5 text-xs font-semibold text-red-500 transition-colors hover:bg-red-50 disabled:opacity-40">
                      {judgeDetachingId === tj.id ? "Detaching…" : "Detach"}
                    </button>
                  </div>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      {#if judgeUpdateError}
        <p class="mb-3 text-xs text-brand-red">{judgeUpdateError}</p>
      {/if}
    {:else}
      <p class="mb-4 text-sm text-brand-muted">No judges attached yet.</p>
    {/if}

    {#if unattachedJudges.length > 0}
      <div class="rounded-[8px] border border-brand-border p-4">
        <div class="grid grid-cols-2 gap-3">
          <div>
            <label class="mb-1 block text-xs font-semibold text-brand-text" for="judge-select-drawer">Attach judge</label>
            <select id="judge-select-drawer" value={selectedJudgeId}
              onchange={(e) => { selectedJudgeId = (e.target as HTMLSelectElement).value; }}
              class="h-[36px] w-full rounded-[6px] border border-brand-border bg-white px-2 text-xs text-brand-text focus:border-brand-blue focus:outline-none">
              <option value="">Select a judge…</option>
              {#each unattachedJudges as j (j.id)}
                <option value={j.id}>{j.name} ({j.slug}) — {scaleSummary(j.rating_scale)}</option>
              {/each}
            </select>
          </div>
          <div>
            <label class="mb-1 block text-xs font-semibold text-brand-text" for="judge-ordinal-drawer">Ordinal</label>
            <input id="judge-ordinal-drawer" type="number" min="0" value={judgeOrdinalInput}
              oninput={(e) => { judgeOrdinalInput = (e.target as HTMLInputElement).value; }}
              class="h-[36px] w-full rounded-[6px] border border-brand-border px-2 text-xs text-brand-text focus:border-brand-blue focus:outline-none" />
          </div>
        </div>
        <label class="mt-3 flex items-center gap-2">
          <input type="checkbox" checked={judgeUseOverride}
            onchange={(e) => { judgeUseOverride = (e.target as HTMLInputElement).checked; }} />
          <span class="text-xs font-semibold text-brand-text">
            Override rating scale <span class="font-normal text-brand-muted">(else uses judge default)</span>
          </span>
        </label>
        {#if judgeUseOverride}
          <div class="mt-2 grid grid-cols-3 gap-2">
            <input type="number" step="0.1" placeholder="min" value={judgeOvMin}
              oninput={(e) => { judgeOvMin = (e.target as HTMLInputElement).value; }}
              class="h-[32px] rounded-[6px] border border-brand-border px-2 text-xs text-brand-text focus:border-brand-blue focus:outline-none" />
            <input type="number" step="0.1" placeholder="max" value={judgeOvMax}
              oninput={(e) => { judgeOvMax = (e.target as HTMLInputElement).value; }}
              class="h-[32px] rounded-[6px] border border-brand-border px-2 text-xs text-brand-text focus:border-brand-blue focus:outline-none" />
            <input type="number" step="0.1" placeholder="step" value={judgeOvStep}
              oninput={(e) => { judgeOvStep = (e.target as HTMLInputElement).value; }}
              class="h-[32px] rounded-[6px] border border-brand-border px-2 text-xs text-brand-text focus:border-brand-blue focus:outline-none" />
          </div>
        {/if}
        {#if judgeAttachError}
          <p class="mt-2 text-xs text-brand-red">{judgeAttachError}</p>
        {/if}
        <button type="button" onclick={() => onJudgeAttach()} disabled={judgeAttaching}
          class="mt-3 rounded-btn bg-brand-blue px-4 py-1.5 text-xs font-semibold text-white transition-opacity hover:opacity-80 disabled:opacity-40">
          {judgeAttaching ? "Attaching…" : "Attach judge"}
        </button>
      </div>
    {/if}
  </div>
{/if}
