<script lang="ts">
  import { ikAvatar } from '$lib/imagekit';
  import type { SessionPlayerStatsResponse } from "$lib/api/types";

  let { stats }: { stats: SessionPlayerStatsResponse | null } = $props();

  function fmtTokens(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
    return String(n);
  }

  /** Agents actually observed in reported stats, falling back to the
   * metadata-declared agent when nothing reported yet. */
  function agentLabel(p: SessionPlayerStatsResponse["players"][number]): string | null {
    if (p.agents.length > 0) return p.agents.join(", ");
    return p.agent_display_name;
  }
</script>

{#if stats && stats.players.length > 0}
  <div class="mb-[20px] rounded-[8px] bg-white px-[16px] py-[20px] sm:px-[24px]">
    <h2 class="mb-[16px] font-heading text-[16px] font-bold" style="color: #363636;">Statistics</h2>
    <div class="overflow-x-auto">
      <table class="w-full border-collapse text-[12px]" style="color: #363636;">
        <thead>
          <tr
            class="text-left text-[10px] font-semibold uppercase tracking-wide"
            style="color: #9ea7b6;"
          >
            <th class="py-[8px] pr-[16px] font-semibold">Player</th>
            <th class="py-[8px] pr-[16px] font-semibold">Points</th>
            <th class="py-[8px] pr-[16px] font-semibold">Tasks</th>
            <th class="py-[8px] pr-[16px] font-semibold">Probes</th>
            <th class="py-[8px] pr-[16px] font-semibold">Agent</th>
            <th class="py-[8px] pr-[16px] font-semibold">Model</th>
            <th class="py-[8px] pr-[16px] font-semibold">Tokens</th>
            <th class="py-[8px] pr-[16px] font-semibold">Tool calls</th>
            <th class="py-[8px] font-semibold">Est. cost</th>
          </tr>
        </thead>
        <tbody>
          {#each stats.players as p (p.player_id)}
            <tr class="border-t align-middle" style="border-color: #eef3fb;">
              <td class="py-[10px] pr-[16px]">
                <div class="flex items-center gap-[10px] whitespace-nowrap">
                  <span class="w-[22px] shrink-0 text-[13px] font-bold" style="color: #9ea7b6;">
                    #{p.rank}
                  </span>
                  {#if p.avatar_url}
                    <img
                      src={ikAvatar(p.avatar_url, 28)}
                      alt=""
                      class="h-[28px] w-[28px] shrink-0 rounded-full object-cover"
                    />
                  {:else}
                    <span
                      class="flex h-[28px] w-[28px] shrink-0 items-center justify-center rounded-full bg-[#dce9fc] text-[12px] font-bold text-[#36547f]"
                    >
                      {p.display_name.slice(0, 1).toUpperCase()}
                    </span>
                  {/if}
                  <span class="text-[13px] font-semibold">{p.display_name}</span>
                </div>
              </td>
              <td class="py-[10px] pr-[16px]">
                <div
                  class="whitespace-nowrap text-[13px] font-bold"
                  style="color: {p.game_points < 0 ? '#e5484d' : '#0269fb'};"
                >
                  {p.game_points}
                </div>
                <div class="whitespace-nowrap text-[11px]" style="color: #9ea7b6;">
                  {p.probe_points} probes · {p.bonus_points} bonus · {p.judge_points} judges
                </div>
              </td>
              <td class="whitespace-nowrap py-[10px] pr-[16px] font-bold">
                {p.solved_tasks}{#if stats.total_tasks > 0}<span
                    class="font-normal"
                    style="color: #9ea7b6;">/{stats.total_tasks}</span
                  >{/if}
              </td>
              <td class="whitespace-nowrap py-[10px] pr-[16px] font-bold">{p.probes}</td>
              <td class="whitespace-nowrap py-[10px] pr-[16px]">
                {#if agentLabel(p)}
                  <span class="rounded-full bg-[#eef4fd] px-2 py-px font-semibold text-[#36547f]">
                    {agentLabel(p)}
                  </span>
                {:else}
                  <span style="color: #9ea7b6;">—</span>
                {/if}
              </td>
              <td class="py-[10px] pr-[16px]">
                {#if p.models.length > 0}
                  <div class="flex flex-wrap gap-[4px]">
                    {#each p.models as m (m)}
                      <span class="whitespace-nowrap rounded-full bg-[#f4f8fe] px-2 py-px text-[#36547f]">
                        {m}
                      </span>
                    {/each}
                  </div>
                {:else}
                  <span style="color: #9ea7b6;">—</span>
                {/if}
              </td>
              <td class="whitespace-nowrap py-[10px] pr-[16px]">
                {#if p.input_tokens + p.cache_read_tokens + p.cache_write_tokens + p.output_tokens > 0}
                  <span class="font-bold"
                    >{fmtTokens(p.input_tokens + p.cache_read_tokens + p.cache_write_tokens)}</span
                  >
                  <span style="color: #9ea7b6;">in</span>
                  · <span class="font-bold">{fmtTokens(p.output_tokens)}</span>
                  <span style="color: #9ea7b6;">out</span>
                {:else}
                  <span style="color: #9ea7b6;">—</span>
                {/if}
              </td>
              <td class="whitespace-nowrap py-[10px] pr-[16px]">
                {#if p.tool_calls > 0}
                  <span class="font-bold">{p.tool_calls}</span>
                {:else}
                  <span style="color: #9ea7b6;">—</span>
                {/if}
              </td>
              <td class="whitespace-nowrap py-[10px]">
                {#if p.cost !== null && p.cost > 0}
                  <span class="font-bold">${p.cost.toFixed(2)}</span>
                {:else}
                  <span style="color: #9ea7b6;">—</span>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  </div>
{/if}
