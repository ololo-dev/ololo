<!--
  Full information about one check, shown in the chat bubble's hover card:
  every run of the check pageable oldest→newest, the command it executed,
  the exact expected/got values, the fixtures it was rendered with, and the
  run's vitals. A copy button puts the visible run on the clipboard so a
  player can hand it to an agent or a bug report without retyping.
-->
<script lang="ts">
  import { Check, ChevronLeft, ChevronRight, Copy } from 'lucide-svelte'
  import type { PlayerProbeEntry } from '$lib/types/arena'
  import { actualOf, expectedOf, probeStatus, shownValue } from '$lib/sessions/probe-briefing'

  interface Props {
    /** Every run of this check, oldest first; the newest opens selected. */
    attempts: PlayerProbeEntry[]
    /** Quiz probes ask their question via the probe URL — shown as title. */
    question?: string | null
    /** The check's total score across all runs. */
    points?: number
  }

  let { attempts, question = null, points = 0 }: Props = $props()

  // null = "the newest run", so a run that lands while the card is open
  // keeps the view on the latest instead of freezing on a stale index.
  let selected = $state<number | null>(null)
  const idx = $derived(Math.min(selected ?? attempts.length - 1, attempts.length - 1))
  const probe = $derived(attempts[idx])
  const status = $derived(probeStatus(probe))
  const command = $derived((probe.rendered_command || probe.test_command || '').trim())

  // The command block clamps to a few lines; without a cue a longer command
  // silently reads as the whole thing. Measure the clamped element and offer
  // the rest explicitly.
  let commandEl = $state<HTMLElement | null>(null)
  let commandExpanded = $state(false)
  let commandOverflows = $state(false)
  $effect(() => {
    void command // re-measure when paging switches the probe
    if (!commandExpanded && commandEl) {
      commandOverflows = commandEl.scrollHeight > commandEl.clientHeight + 2
    }
  })

  function timeLabel(iso: string | null): string {
    if (!iso) return ''
    return new Date(iso).toLocaleTimeString([], {
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    })
  }

  function durationLabel(ms: number): string {
    return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`
  }

  function pointsChip(delta: number): string {
    return `${delta > 0 ? '+' : ''}${delta}`
  }

  // Fixture values as label→value rows; null when the payload is not the
  // flat JSON object the engine normally stores.
  function fixtureRows(p: PlayerProbeEntry): [string, string][] | null {
    if (!p.fixture_values) return null
    try {
      const parsed: unknown = JSON.parse(p.fixture_values)
      if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return null
      const rows = Object.entries(parsed).map(
        ([k, v]): [string, string] => [k, typeof v === 'string' ? v : JSON.stringify(v)],
      )
      return rows.length > 0 ? rows : null
    } catch {
      return null
    }
  }

  function select(next: number) {
    selected = Math.min(Math.max(next, 0), attempts.length - 1)
    commandExpanded = false
  }

  // The visible run as markdown — everything the card shows, pasteable.
  function copyText(): string {
    const out = [`### Check: ${question ?? probe.label ?? 'automated check'}`]
    out.push(`- Status: ${status === 'pending' ? 'running' : status === 'pass' ? 'passed' : 'failed'}`)
    if (attempts.length > 1) out.push(`- Run: ${idx + 1} of ${attempts.length}`)
    const at = probe.resolved_at ?? probe.dispatched_at
    if (at) out.push(`- At: ${at}`)
    if (probe.exit_code !== null) out.push(`- Exit code: ${probe.exit_code}`)
    if (probe.duration_ms !== null) out.push(`- Duration: ${durationLabel(probe.duration_ms)}`)
    if (probe.point_delta !== 0) out.push(`- Points: ${pointsChip(probe.point_delta)}`)
    out.push(`- Expected: ${shownValue(expectedOf(probe), 'hidden for this probe type')}`)
    out.push(`- Got: ${shownValue(actualOf(probe), status === 'pending' ? 'still running' : 'no answer')}`)
    const fixtures = fixtureRows(probe)
    if (fixtures) {
      out.push('- Fixtures:')
      for (const [k, v] of fixtures) out.push(`  - ${k}: ${v}`)
    }
    if (command) out.push('', 'Command:', '', '```sh', command, '```')
    return out.join('\n')
  }

  let copyState = $state<'copied' | 'failed' | null>(null)
  let copyTimer: ReturnType<typeof setTimeout> | null = null
  async function copy() {
    if (copyTimer) clearTimeout(copyTimer)
    try {
      await navigator.clipboard.writeText(copyText())
      copyState = 'copied'
    } catch {
      copyState = 'failed'
    }
    copyTimer = setTimeout(() => (copyState = null), 2500)
  }
  $effect(() => () => {
    if (copyTimer) clearTimeout(copyTimer)
  })
</script>

<div class="mb-2 flex items-center gap-2">
  <span class="min-w-0 flex-1 truncate text-[12px] font-semibold text-brand-text">
    {question ?? probe.label ?? 'Automated check'}
  </span>
  <span
    class="shrink-0 rounded-full px-2 py-0.5 text-[10px] font-bold uppercase tracking-wider {status === 'pass'
      ? 'bg-green-100 text-green-700'
      : status === 'fail'
        ? 'bg-red-100 text-brand-red'
        : 'bg-brand-light-blue text-brand-blue'}"
  >
    {status === 'pending' ? 'running' : status === 'pass' ? 'passed' : 'failed'}
  </span>
</div>

<div class="mb-2 flex items-center gap-1.5">
  {#if attempts.length > 1}
    <button
      type="button"
      onclick={() => select(idx - 1)}
      disabled={idx === 0}
      title="Previous run"
      data-testid="check-hover-prev"
      class="inline-flex h-5 w-5 items-center justify-center rounded border border-brand-border/60 text-brand-text transition-colors enabled:hover:border-brand-blue enabled:hover:text-brand-blue disabled:opacity-40"
    >
      <ChevronLeft size={12} />
    </button>
    <span class="text-[11px] tabular-nums text-brand-muted" data-testid="check-hover-run-label">
      run {idx + 1} of {attempts.length}
    </span>
    <button
      type="button"
      onclick={() => select(idx + 1)}
      disabled={idx === attempts.length - 1}
      title="Next run"
      data-testid="check-hover-next"
      class="inline-flex h-5 w-5 items-center justify-center rounded border border-brand-border/60 text-brand-text transition-colors enabled:hover:border-brand-blue enabled:hover:text-brand-blue disabled:opacity-40"
    >
      <ChevronRight size={12} />
    </button>
  {/if}
  <button
    type="button"
    onclick={copy}
    title="Copy this run's details as markdown"
    data-testid="check-hover-copy"
    class="ml-auto inline-flex items-center gap-1 rounded border border-brand-border/60 px-1.5 py-0.5 text-[10px] font-semibold text-brand-text transition-colors hover:border-brand-blue hover:text-brand-blue"
  >
    {#if copyState === 'copied'}
      <Check size={11} /> Copied
    {:else if copyState === 'failed'}
      <Copy size={11} /> Copy failed
    {:else}
      <Copy size={11} /> Copy
    {/if}
  </button>
</div>

{#if command}
  <p class="mb-0.5 text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Command</p>
  <div class="relative mb-2">
    <pre
      bind:this={commandEl}
      class="whitespace-pre-wrap break-all rounded bg-brand-light-blue/50 px-2 py-1.5 font-mono text-[11px] leading-relaxed text-brand-text {commandExpanded
        ? 'max-h-56 overflow-y-auto'
        : 'max-h-[4.25rem] overflow-hidden'}">{command}</pre>
    {#if !commandExpanded && commandOverflows}
      <!-- The fade admits the cut; the button hands over the rest. -->
      <div
        class="pointer-events-none absolute inset-x-0 bottom-0 flex h-9 items-end justify-center rounded-b bg-gradient-to-t from-white via-white/80 to-transparent"
      >
        <button
          type="button"
          onclick={() => (commandExpanded = true)}
          data-testid="check-hover-expand-command"
          class="pointer-events-auto mb-0.5 rounded border border-brand-border/60 bg-white px-1.5 py-px text-[10px] font-semibold text-brand-blue hover:border-brand-blue"
        >
          Show full command
        </button>
      </div>
    {/if}
  </div>
{/if}

<div class="mb-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-[12px]">
  <span class="font-semibold text-brand-muted">Expected</span>
  <span class="min-w-0 break-all font-mono text-[11px] text-brand-text">
    {shownValue(expectedOf(probe), 'hidden for this probe type')}
  </span>
  <span class="font-semibold text-brand-muted">Got</span>
  <span class="min-w-0 break-all font-mono text-[11px] text-brand-text">
    {shownValue(actualOf(probe), status === 'pending' ? 'still running' : 'no answer')}
  </span>
</div>

{#if fixtureRows(probe)}
  <p class="mb-0.5 text-[10px] font-semibold uppercase tracking-wider text-brand-muted">Fixtures</p>
  <div class="mb-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-[11px]">
    {#each fixtureRows(probe) ?? [] as [key, value] (key)}
      <span class="font-mono text-brand-muted">{key}</span>
      <span class="min-w-0 break-all font-mono text-brand-text">{value}</span>
    {/each}
  </div>
{/if}

<div class="flex flex-wrap items-center gap-x-3 gap-y-0.5 border-t border-brand-border/40 pt-1.5 text-[11px] text-brand-muted">
  {#if probe.exit_code !== null}<span>exit {probe.exit_code}</span>{/if}
  {#if probe.duration_ms !== null}<span>{durationLabel(probe.duration_ms)}</span>{/if}
  <span>{timeLabel(probe.resolved_at ?? probe.dispatched_at)}</span>
  {#if probe.point_delta !== 0}
    <span class="font-bold tabular-nums {probe.point_delta > 0 ? 'text-green-600' : 'text-brand-red'}">
      {pointsChip(probe.point_delta)} pts
    </span>
  {/if}
  {#if attempts.length > 1 && points !== probe.point_delta && points !== 0}
    <span class="ml-auto tabular-nums">total <span class="font-bold {points > 0 ? 'text-green-600' : 'text-brand-red'}">{pointsChip(points)} pts</span></span>
  {/if}
</div>
