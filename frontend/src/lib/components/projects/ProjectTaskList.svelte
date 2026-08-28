<script lang="ts">
  import { LinkPreview } from "bits-ui";
  import type { TaskPreviewItem, TaskPreviewJudge } from "$lib/api";
  import TypewriterMarkdown from "$lib/components/sessions/TypewriterMarkdown.svelte";
  import { ikAvatar } from "$lib/imagekit";

  let { tasks }: { tasks: TaskPreviewItem[] } = $props();

  // One honest sentence about how the task pays. `points` means two very
  // different things (audit follow-up: the bare "+200 pts" chip): a classic
  // task pays it per passing check, an open-ended task hands it to the judge
  // panel as the budget their 0–10 verdicts map onto.
  function scoringLine(t: TaskPreviewItem): string {
    if (t.open_ended) {
      const n = t.judges?.length ?? 0;
      return n > 1
        ? `Open-ended — a panel of ${n} judges splits a ${t.points}-pt budget`
        : `Open-ended — judged against a ${t.points}-pt budget`;
    }
    const bonus = t.completion_bonus ?? 0;
    const base = `+${t.points} pts per passing check`;
    return bonus > 0 ? `${base} · +${bonus} for completing the task` : base;
  }

  function initial(name: string): string {
    return name.trim().charAt(0).toUpperCase() || "?";
  }
</script>

{#snippet judgeCard(judge: TaskPreviewJudge)}
  <div class="flex items-center gap-3">
    {#if judge.avatar_url}
      <img
        src={ikAvatar(judge.avatar_url, 40)}
        alt=""
        class="h-[40px] w-[40px] rounded-full object-cover"
      />
    {:else}
      <span
        class="flex h-[40px] w-[40px] items-center justify-center rounded-full bg-brand-blue/15 text-[16px] font-bold text-brand-blue"
      >{initial(judge.name)}</span>
    {/if}
    <div class="min-w-0">
      <div class="text-[15px] font-bold text-brand-text">{judge.name}</div>
      <div class="font-mono text-[11px] text-brand-muted">{judge.slug}</div>
    </div>
  </div>
  {#if judge.description}
    <p class="mt-3 text-[13px] leading-relaxed text-brand-text/80">
      {judge.description}
    </p>
  {/if}
{/snippet}

<!-- The task arc a player signs up for (audit UI-H2), styled as the same
     card rows the Top Players and Sessions tabs use. Native <details> keeps
     the briefs collapsible without JS, so SSR and no-script agents read the
     full arc. Projects that stage their tasks as a surprise
     (show_tasks: false) never receive this list from the server. -->
{#if tasks.length === 0}
  <p class="mt-[16px] text-brand-muted">This project keeps its tasks hidden until you play.</p>
{:else}
  <ul class="mt-[16px] flex flex-col gap-[8px]">
    {#each tasks as task, i (task.ordinal)}
      <li>
        <details class="group overflow-hidden rounded-[8px] bg-white">
          <!-- On phones the row folds in two: number + title + points stay on
               the first line, the scoring note drops to its own full-width
               line, and the judge faces hide — otherwise the title truncated
               to a couple of characters. -->
          <summary
            class="flex cursor-pointer list-none flex-wrap items-center gap-x-[12px] gap-y-[6px] px-[16px] py-[14px] transition-colors hover:bg-[#f9fbff] sm:flex-nowrap sm:gap-[16px] sm:px-[24px] sm:py-[16px] [&::-webkit-details-marker]:hidden"
          >
            <span
              class="flex h-[32px] w-[32px] shrink-0 items-center justify-center rounded-full text-[14px] font-bold tabular-nums"
              style="background: #e2ecfc; color: #8fb4ec;"
            >
              {i + 1}
            </span>

            <div class="min-w-0 flex-1">
              <p class="truncate font-semibold text-brand-text">{task.title}</p>
              <p class="hidden text-[13px] text-brand-muted sm:block">{scoringLine(task)}</p>
            </div>

            {#if (task.judges?.length ?? 0) > 0}
              <!-- Who grades this task: each face opens a hover card with the
                   judge's blurb. Clicks are swallowed so peeking at a judge
                   never toggles the accordion. -->
              <span
                class="hidden shrink-0 -space-x-[8px] sm:flex"
                role="presentation"
                onclick={(e) => e.preventDefault()}
              >
                {#each task.judges!.slice(0, 5) as judge (judge.slug)}
                  <LinkPreview.Root openDelay={120} closeDelay={80}>
                    <LinkPreview.Trigger
                      class="block cursor-default rounded-full transition-transform hover:z-10 hover:scale-110 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-brand-blue/60"
                    >
                      {#if judge.avatar_url}
                        <img
                          src={ikAvatar(judge.avatar_url, 32)}
                          alt={judge.name}
                          class="h-[32px] w-[32px] rounded-full border-2 border-white object-cover"
                        />
                      {:else}
                        <span
                          class="flex h-[32px] w-[32px] items-center justify-center rounded-full border-2 border-white bg-brand-light-blue text-[13px] font-bold text-brand-blue"
                        >{initial(judge.name)}</span>
                      {/if}
                    </LinkPreview.Trigger>
                    <LinkPreview.Content
                      sideOffset={8}
                      class="z-50 w-[300px] rounded-[10px] border border-brand-border bg-white p-4 text-left shadow-[0_12px_32px_rgba(15,23,42,0.18)]"
                    >
                      {@render judgeCard(judge)}
                    </LinkPreview.Content>
                  </LinkPreview.Root>
                {/each}
                {#if task.judges!.length > 5}
                  <span
                    class="flex h-[32px] w-[32px] items-center justify-center rounded-full border-2 border-white bg-brand-light-blue text-[11px] font-bold text-brand-muted"
                  >+{task.judges!.length - 5}</span>
                {/if}
              </span>
            {/if}

            <div class="shrink-0 text-right">
              <p class="text-[20px] font-bold tabular-nums" style="color: #363636;">
                {task.points}
              </p>
              <p class="text-[12px] font-semibold uppercase tracking-wide text-brand-muted">
                {task.open_ended ? "pt budget" : "pts / check"}
              </p>
            </div>

            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="16"
              height="16"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2.5"
              stroke-linecap="round"
              stroke-linejoin="round"
              class="shrink-0 text-brand-muted transition-transform group-open:rotate-90"
            >
              <polyline points="9 18 15 12 9 6" />
            </svg>

            <!-- Phone-only second line: the scoring note gets the full width
                 the first line could not spare. -->
            <p class="w-full pl-[44px] text-[12px] leading-snug text-brand-muted sm:hidden">
              {scoringLine(task)}
            </p>
          </summary>

          <div class="border-t border-brand-light-blue px-[16px] py-[16px] sm:px-[24px] sm:py-[20px]">
            <!-- Briefs are loosely-authored markdown (pipe tables without
                 separator rows, Gherkin scenarios on single line breaks) —
                 the chat's normalizer renders them faithfully. -->
            <TypewriterMarkdown value={task.description} class="text-[14px]" />

            {#if (task.judges?.length ?? 0) > 0}
              <div class="mt-[18px] flex flex-wrap items-center gap-[8px] border-t border-brand-light-blue pt-[14px]">
                <span class="text-[12px] font-semibold uppercase tracking-wide text-brand-muted">
                  Judged by
                </span>
                {#each task.judges! as judge (judge.slug)}
                  <LinkPreview.Root openDelay={120} closeDelay={80}>
                    <LinkPreview.Trigger
                      class="flex cursor-default items-center gap-[6px] rounded-full bg-brand-light-blue py-[3px] pl-[3px] pr-[10px] text-[13px] font-semibold text-brand-text transition-colors hover:bg-[#d0e2fb] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-brand-blue/60"
                    >
                      {#if judge.avatar_url}
                        <img
                          src={ikAvatar(judge.avatar_url, 24)}
                          alt=""
                          class="h-[24px] w-[24px] rounded-full object-cover"
                        />
                      {:else}
                        <span
                          class="flex h-[24px] w-[24px] items-center justify-center rounded-full bg-white text-[11px] font-bold text-brand-blue"
                        >{initial(judge.name)}</span>
                      {/if}
                      {judge.name}
                    </LinkPreview.Trigger>
                    <LinkPreview.Content
                      sideOffset={8}
                      class="z-50 w-[300px] rounded-[10px] border border-brand-border bg-white p-4 text-left shadow-[0_12px_32px_rgba(15,23,42,0.18)]"
                    >
                      {@render judgeCard(judge)}
                    </LinkPreview.Content>
                  </LinkPreview.Root>
                {/each}
              </div>
            {/if}
          </div>
        </details>
      </li>
    {/each}
  </ul>
{/if}
