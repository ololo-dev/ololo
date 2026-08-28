<script lang="ts">
  // Langfuse-style observation timeline for one LLM request: a chronological
  // row per turn (llm / tool) with a proportional duration bar, expandable
  // into the prompt, completion, tool args, error and full transcript.

  import type { JudgeLogEvent } from '$lib/types/arena';
  import { formatTokens } from '$lib/format';

  let { events }: { events: JudgeLogEvent[] } = $props();

  // The enveloping LLM event is recorded when the agent loop finishes but
  // stamped with its start, so raw array order can put it after its children.
  const ordered = $derived([...events].sort((a, b) => a.at_ms - b.at_ms));
  const firstAt = $derived(ordered.length > 0 ? Math.min(...ordered.map((e) => e.at_ms)) : 0);
  const maxDuration = $derived(Math.max(1, ...ordered.map((e) => e.duration_ms || 0)));

  let open = $state(new Set<number>());
  let rawMessages = $state(new Set<number>());
  let copied = $state<string | null>(null);

  function toggle(i: number) {
    const next = new Set(open);
    if (next.has(i)) next.delete(i);
    else next.add(i);
    open = next;
  }

  function toggleRaw(i: number) {
    const next = new Set(rawMessages);
    if (next.has(i)) next.delete(i);
    else next.add(i);
    rawMessages = next;
  }

  async function copy(key: string, text: string) {
    try {
      await navigator.clipboard.writeText(text);
      copied = key;
      setTimeout(() => {
        if (copied === key) copied = null;
      }, 1500);
    } catch {
      // Clipboard unavailable (insecure context / denied) — silently ignore.
    }
  }

  function offset(at: number): string {
    return `+${((at - firstAt) / 1000).toFixed(1)}s`;
  }

  function barPct(ms: number): number {
    return Math.max(2, Math.round(((ms || 0) / maxDuration) * 100));
  }

  function eventTitle(ev: JudgeLogEvent): string {
    // A judge run is not only model turns: the evidence snapshot it read, the
    // decision its own program made, and each probe a sandbox re-ran are
    // stages too, and they carry their name rather than a model id.
    if (ev.name) return ev.name;
    if (ev.kind === 'tool') return 'tool';
    return ev.model ?? 'LLM turn';
  }

  /** Badge tint per stage kind — model turns and tool calls keep theirs. */
  function kindClass(kind: string): string {
    if (kind === 'tool') return 'bg-amber-100 text-amber-700';
    if (kind === 'llm') return 'bg-brand-blue/10 text-brand-blue';
    return 'bg-brand-light-blue text-brand-muted';
  }

  function outputLabel(kind: string): string {
    if (kind === 'tool') return 'Output (tool result)';
    if (kind === 'llm') return 'Output (completion)';
    return 'Output';
  }

  function hasTokens(ev: JudgeLogEvent): boolean {
    return ev.tokens_input != null || ev.tokens_output != null;
  }

  /** Pretty-print tool args when they parse as JSON; otherwise pass through. */
  function prettyArgs(args: string): string {
    try {
      return JSON.stringify(JSON.parse(args), null, 2);
    } catch {
      return args;
    }
  }

  // ---------- Transcript normalization ----------
  // `messages` is an opaque serde_json::Value from rig; shapes vary by
  // provider. Everything here is defensive: unknown shapes fall back to the
  // raw JSON view rather than throwing.

  type ToolCall = { name: string; args: string };
  type Turn = { role: string; text: string; calls: ToolCall[] };

  function asRecord(v: unknown): Record<string, unknown> | null {
    return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
  }

  function stringify(v: unknown): string {
    if (v == null) return '';
    if (typeof v === 'string') return v;
    try {
      return JSON.stringify(v, null, 2);
    } catch {
      return String(v);
    }
  }

  function toolCallFrom(part: Record<string, unknown>): ToolCall | null {
    // Shapes seen in the wild: {function:{name,arguments}},
    // {toolCall:{function:{...}}}, {tool_call:{name,arguments}},
    // {type:"tool_use", name, input}.
    const nested =
      asRecord(part.toolCall) ?? asRecord(part.tool_call) ?? asRecord(part.tool_use) ?? part;
    const fn = asRecord(nested.function) ?? nested;
    const name = fn.name ?? nested.name;
    if (typeof name !== 'string') return null;
    const args = fn.arguments ?? fn.args ?? nested.input ?? nested.arguments;
    return { name, args: typeof args === 'string' ? prettyArgs(args) : stringify(args) };
  }

  function normalizeContent(content: unknown): { text: string; calls: ToolCall[] } {
    if (typeof content === 'string') return { text: content, calls: [] };
    const parts = Array.isArray(content) ? content : [content];
    const texts: string[] = [];
    const calls: ToolCall[] = [];
    for (const raw of parts) {
      if (typeof raw === 'string') {
        texts.push(raw);
        continue;
      }
      const part = asRecord(raw);
      if (!part) continue;
      const call = toolCallFrom(part);
      if (call) {
        calls.push(call);
        continue;
      }
      if (typeof part.text === 'string') texts.push(part.text);
      else if (typeof part.content === 'string') texts.push(part.content);
      else texts.push(stringify(part));
    }
    return { text: texts.join('\n').trim(), calls };
  }

  /** Best-effort turn list; null when the payload isn't an array of turns. */
  function transcriptTurns(messages: unknown): Turn[] | null {
    if (!Array.isArray(messages) || messages.length === 0) return null;
    const turns: Turn[] = [];
    for (const raw of messages) {
      const msg = asRecord(raw);
      if (!msg) return null;
      const role = typeof msg.role === 'string' ? msg.role : 'message';
      const { text, calls } = normalizeContent(msg.content ?? msg);
      turns.push({ role, text, calls });
    }
    return turns;
  }

  function roleClass(role: string): string {
    if (role === 'assistant') return 'bg-brand-blue/10 text-brand-blue';
    if (role === 'user') return 'bg-green-100 text-green-700';
    if (role === 'system') return 'bg-gray-100 text-gray-600';
    return 'bg-amber-100 text-amber-700';
  }
</script>

{#snippet section(key: string, label: string, body: string, tone: 'normal' | 'error')}
  <div>
    <div class="mb-1 flex items-center gap-2">
      <span class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">{label}</span>
      <button
        type="button"
        onclick={() => void copy(key, body)}
        class="rounded px-1.5 py-0.5 text-[10px] font-semibold text-brand-blue transition-colors hover:bg-brand-blue/10"
      >{copied === key ? 'Copied' : 'Copy'}</button>
    </div>
    <pre
      class="max-h-96 overflow-auto whitespace-pre-wrap break-words rounded-md border px-3 py-2 font-mono text-[11px]
             {tone === 'error'
               ? 'border-red-200 bg-red-50 text-red-700'
               : 'border-brand-border/40 bg-white text-brand-text'}">{body}</pre>
  </div>
{/snippet}

<div data-testid="trace-timeline">
  <div class="mb-2 flex items-baseline justify-between">
    <h3 class="font-heading text-[10px] font-semibold uppercase tracking-wider text-brand-muted">
      Observations ({ordered.length})
    </h3>
    {#if open.size > 0}
      <button
        type="button"
        onclick={() => { open = new Set(); }}
        class="rounded px-1.5 py-0.5 text-[10px] font-semibold text-brand-blue transition-colors hover:bg-brand-blue/10"
      >Collapse all</button>
    {/if}
  </div>

  <ul class="divide-y divide-brand-border/40 rounded-md border border-brand-border/40 bg-white">
    {#each ordered as ev, i (i)}
      {@const isOpen = open.has(i)}
      <li data-testid="trace-event-{i}">
        <button
          type="button"
          onclick={() => toggle(i)}
          class="flex w-full items-center gap-3 px-3 py-2 text-left transition-colors hover:bg-brand-light-blue/20"
        >
          <span class="w-6 shrink-0 text-right font-mono text-[10px] text-brand-muted">{i + 1}</span>
          <span
            class="shrink-0 rounded-full px-2 py-0.5 text-[10px] font-semibold {kindClass(ev.kind)}"
          >{ev.kind}</span>
          <span class="w-16 shrink-0 font-mono text-[10px] text-brand-muted">{offset(ev.at_ms)}</span>
          <span class="min-w-0 flex-1">
            <span class="block truncate text-[12px] font-semibold text-brand-text">{eventTitle(ev)}</span>
            <span class="mt-1 flex items-center gap-2">
              <span class="h-1.5 flex-1 overflow-hidden rounded-full bg-brand-light-blue">
                <span
                  class="block h-full rounded-full {ev.error ? 'bg-red-400' : 'bg-brand-blue/60'}"
                  style="width: {barPct(ev.duration_ms)}%"
                ></span>
              </span>
              <span class="w-16 shrink-0 text-right font-mono text-[10px] tabular-nums text-brand-muted">
                {(ev.duration_ms / 1000).toFixed(1)}s
              </span>
            </span>
          </span>
          {#if hasTokens(ev)}
            <span class="shrink-0 text-right text-[10px] tabular-nums text-brand-muted">
              {formatTokens(ev.tokens_input)} / {formatTokens(ev.tokens_output)}
              {#if (ev.tokens_cache_read ?? 0) > 0 || (ev.tokens_cache_write ?? 0) > 0}
                <span class="block text-[9px]">
                  cache {formatTokens(ev.tokens_cache_read)} / {formatTokens(ev.tokens_cache_write)}
                </span>
              {/if}
            </span>
          {/if}
          {#if ev.error}
            <span class="shrink-0 rounded-full bg-red-100 px-2 py-0.5 text-[10px] font-semibold text-red-600">error</span>
          {/if}
          <svg
            width="12" height="12" viewBox="0 0 24 24" fill="none" aria-hidden="true"
            class="shrink-0 text-brand-muted transition-transform {isOpen ? 'rotate-180' : ''}"
          >
            <path d="M6 9l6 6 6-6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>

        {#if isOpen}
          {@const turns = transcriptTurns(ev.messages)}
          {@const showRaw = rawMessages.has(i) || turns === null}
          <div class="space-y-3 border-t border-brand-border/40 bg-brand-light-blue/10 px-3 py-3">
            {#if ev.output_chars != null}
              <p class="text-[10px] text-brand-muted">Output size: {ev.output_chars} characters</p>
            {/if}
            {#if ev.error}
              {@render section(`err-${i}`, 'Error', ev.error, 'error')}
            {/if}
            {#if ev.args}
              {@render section(`args-${i}`, 'Args', prettyArgs(ev.args), 'normal')}
            {/if}
            {#if ev.input}
              {@render section(`in-${i}`, ev.kind === 'tool' ? 'Input' : 'Input (prompt)', ev.input, 'normal')}
            {/if}
            {#if ev.output}
              {@render section(`out-${i}`, outputLabel(ev.kind), ev.output, 'normal')}
            {/if}

            {#if ev.messages != null}
              <div>
                <div class="mb-1 flex items-center gap-2">
                  <span class="text-[10px] font-semibold uppercase tracking-wider text-brand-muted">
                    Transcript{turns ? ` (${turns.length} turns)` : ''}
                  </span>
                  {#if turns !== null}
                    <button
                      type="button"
                      onclick={() => toggleRaw(i)}
                      class="rounded px-1.5 py-0.5 text-[10px] font-semibold text-brand-blue transition-colors hover:bg-brand-blue/10"
                    >{showRaw ? 'Readable' : 'Raw JSON'}</button>
                  {/if}
                  <button
                    type="button"
                    onclick={() => void copy(`msg-${i}`, stringify(ev.messages))}
                    class="rounded px-1.5 py-0.5 text-[10px] font-semibold text-brand-blue transition-colors hover:bg-brand-blue/10"
                  >{copied === `msg-${i}` ? 'Copied' : 'Copy'}</button>
                </div>
                {#if showRaw || turns === null}
                  <pre class="max-h-96 overflow-auto whitespace-pre-wrap break-words rounded-md border border-brand-border/40 bg-white px-3 py-2 font-mono text-[11px] text-brand-text">{stringify(ev.messages)}</pre>
                {:else}
                  <ul class="max-h-96 space-y-2 overflow-auto rounded-md border border-brand-border/40 bg-white px-3 py-2">
                    {#each turns as turn, ti (ti)}
                      <li class="border-b border-brand-border/20 pb-2 last:border-0 last:pb-0">
                        <span class="rounded-full px-2 py-0.5 text-[10px] font-semibold {roleClass(turn.role)}">
                          {turn.role}
                        </span>
                        {#if turn.text}
                          <pre class="mt-1 max-h-64 overflow-auto whitespace-pre-wrap break-words font-mono text-[11px] text-brand-text">{turn.text}</pre>
                        {/if}
                        {#each turn.calls as call, ci (ci)}
                          <div class="mt-1 rounded border border-amber-200 bg-amber-50 px-2 py-1">
                            <span class="font-mono text-[10px] font-semibold text-amber-700">{call.name}()</span>
                            {#if call.args}
                              <pre class="mt-0.5 max-h-40 overflow-auto whitespace-pre-wrap break-words font-mono text-[10px] text-amber-900">{call.args}</pre>
                            {/if}
                          </div>
                        {/each}
                      </li>
                    {/each}
                  </ul>
                {/if}
              </div>
            {/if}

            {#if !ev.input && !ev.output && !ev.args && !ev.error && ev.messages == null}
              <p class="text-[11px] text-brand-muted">No payload captured for this observation.</p>
            {/if}
          </div>
        {/if}
      </li>
    {/each}
  </ul>
</div>
