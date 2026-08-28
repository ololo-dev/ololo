<script lang="ts">
  import type { PlayerArtifactRequest } from '$lib/types/arena'

  let {
    requests,
    now = Date.now(),
  }: { requests: PlayerArtifactRequest[]; now?: number } = $props()

  function remaining(deadline: string): string {
    const secs = Math.max(0, Math.floor((new Date(deadline).getTime() - now) / 1000))
    const m = Math.floor(secs / 60)
    const s = secs % 60
    return `${m}:${String(s).padStart(2, '0')}`
  }
</script>

{#each requests as req (req.probe_id)}
  <div
    class="flex items-start gap-3 rounded-lg border border-amber-400/60 bg-amber-50 p-3 dark:bg-amber-950/30"
    data-testid="artifact-banner"
    role="alert"
  >
    <span class="mt-0.5 text-amber-600" aria-hidden="true">
      <svg viewBox="0 0 20 20" class="h-5 w-5 fill-current"><path d="M10 2a8 8 0 100 16 8 8 0 000-16zm0 5a1 1 0 011 1v3a1 1 0 11-2 0V8a1 1 0 011-1zm0 7.5a1.25 1.25 0 110-2.5 1.25 1.25 0 010 2.5z"/></svg>
    </span>
    <div class="min-w-0 flex-1">
      <div class="font-heading text-[14px] font-semibold text-amber-800 dark:text-amber-200">
        A judge is waiting for an artifact
      </div>
      <p class="mt-0.5 text-[13px] text-amber-900/90 dark:text-amber-100/80">{req.instruction}</p>
      <p class="mt-1 text-[12px] text-amber-700 dark:text-amber-300">
        Save it under <code class="rounded bg-amber-100 px-1 dark:bg-amber-900/50">{req.path}</code>
        in your working directory — ololo commits and pushes it automatically.
      </p>
    </div>
    <span class="shrink-0 text-[13px] font-semibold tabular-nums text-amber-700 dark:text-amber-300">
      {remaining(req.deadline_at)}
    </span>
  </div>
{/each}
