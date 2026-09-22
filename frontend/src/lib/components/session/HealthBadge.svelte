<script lang="ts">
  import { LEVEL_COLORS, formatScore, type HealthIndicator } from "$lib/session-health";

  /** The code-health chip next to a participant: the latest server-verified
   * score, coloured by level, with the trend against the previous check.
   * Direction is spelled by a glyph, never by colour alone. */
  let {
    indicator,
    compact = true,
  }: {
    indicator: HealthIndicator | null;
    /** The dashboard row chip (tiny) vs. the player-page tile. */
    compact?: boolean;
  } = $props();

  const trendGlyph = $derived(
    indicator?.trend === "up" ? "▲" : indicator?.trend === "down" ? "▼" : indicator?.trend === "flat" ? "–" : "",
  );
  const title = $derived.by(() => {
    if (!indicator) return "";
    const base = indicator.score == null
      ? "Code health: no score yet"
      : `Code health: ${formatScore(indicator.score)} (${indicator.level})`;
    const verified = indicator.verified ? "server-verified" : "client-reported, verification pending";
    const trend =
      indicator.trend === "up"
        ? "up since the previous check"
        : indicator.trend === "down"
          ? "down since the previous check"
          : indicator.trend === "flat"
            ? "unchanged since the previous check"
            : "first check";
    return `${base} — ${verified}; ${trend}`;
  });
</script>

{#if indicator}
  {@const colors = LEVEL_COLORS[indicator.level]}
  <span
    class="inline-flex shrink-0 items-center gap-[3px] whitespace-nowrap rounded-full font-semibold {compact
      ? 'px-[6px] py-[1px] text-[10px]'
      : 'px-2.5 py-0.5 text-[11px]'}"
    style="background: {colors.bg}; color: {colors.fg};"
    {title}
    data-testid="health-badge"
    data-level={indicator.level}
  >
    <span
      class="inline-block rounded-full {compact ? 'h-[6px] w-[6px]' : 'h-[8px] w-[8px]'} {indicator.verified
        ? ''
        : 'border'}"
      style="background: {indicator.verified ? colors.fg : 'transparent'}; border-color: {colors.fg};"
      aria-hidden="true"
    ></span>
    <span>{formatScore(indicator.score)}</span>
    {#if trendGlyph}
      <span aria-label={indicator.trend ?? undefined}>{trendGlyph}</span>
    {/if}
  </span>
{/if}
