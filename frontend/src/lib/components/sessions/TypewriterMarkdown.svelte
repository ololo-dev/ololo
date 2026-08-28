<script lang="ts">
  import { untrack } from 'svelte'
  import MarkdownContent from '$lib/components/MarkdownContent.svelte'

  /**
   * Markdown for chat bubbles.
   *
   * Rendering: bubbles carry loosely-authored text — briefs with pipe tables
   * that have no separator row, Gherkin scenarios that rely on single line
   * breaks. Rendered strictly, both collapse into one flat paragraph, so the
   * value is normalized (a separator row is injected under a header-less
   * table) and parsed with GFM line breaks on.
   *
   * Streaming: text can arrive the way AI chats stream it, revealed in small
   * token-sized chunks. `animate` is sampled once, at mount — a message that
   * started typing keeps typing even when unrelated state re-renders the
   * parent, and already-read messages never re-animate.
   */
  let {
    value,
    animate = false,
    class: extraClass = '',
  }: { value: string; animate?: boolean; class?: string } = $props()

  const isRow = (line: string): boolean => {
    const t = line.trim()
    return t.length > 2 && t.startsWith('|') && t.endsWith('|')
  }
  const isSeparator = (line: string): boolean => /^\|(\s*:?-+:?\s*\|)+$/.test(line.trim())

  /** Inject the `| --- |` row a pipe table needs when the author skipped it. */
  function normalizeLooseMarkdown(src: string): string {
    const lines = src.split('\n')
    const out: string[] = []
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i]
      out.push(line)
      const tableStart =
        isRow(line) && !isSeparator(line) && (i === 0 || !isRow(lines[i - 1]))
      const next = lines[i + 1]
      if (tableStart && next !== undefined && isRow(next) && !isSeparator(next)) {
        const indent = line.slice(0, line.length - line.trimStart().length)
        const cols = line.trim().split('|').length - 2
        out.push(`${indent}|${Array(cols).fill(' --- ').join('|')}|`)
      }
    }
    return out.join('\n')
  }

  const normalized = $derived(normalizeLooseMarkdown(value))

  // svelte-ignore state_referenced_locally — capturing the initial value is
  // the point: a message that started typing keeps typing, and a message
  // rendered as already-read never re-animates.
  const shouldAnimate = animate

  let revealed = $state(shouldAnimate ? 0 : Number.MAX_SAFE_INTEGER)

  $effect(() => {
    if (!shouldAnimate) return
    const total = normalized.length
    // Restarts only when the text itself changes (streamed growth), not on
    // every revealed tick — untrack keeps the counter out of the deps.
    let i = untrack(() => revealed)
    if (i >= total) return
    const timer = setInterval(() => {
      // 2–7 chars per tick reads as tokens, not as a character counter.
      i = Math.min(total, i + 2 + Math.floor(Math.random() * 6))
      revealed = i
      if (i >= total) clearInterval(timer)
    }, 24)
    return () => clearInterval(timer)
  })

  const shown = $derived(revealed >= normalized.length ? normalized : normalized.slice(0, revealed))
</script>

<MarkdownContent value={shown} breaks class={extraClass} />
