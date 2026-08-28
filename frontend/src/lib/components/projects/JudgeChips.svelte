<script lang="ts">
  import { LinkPreview } from 'bits-ui';
  import { ikAvatar } from '$lib/imagekit';
  import type { ProjectJudge } from '$lib/api';

  let { judges }: { judges: ProjectJudge[] } = $props();

  function initial(name: string): string {
    return name.trim().charAt(0).toUpperCase() || '?';
  }
</script>

<!--
  Judges as chips (avatar + name), each opening a hover card with the judge's
  full description — the shadcn-svelte HoverCard pattern, built on bits-ui's
  LinkPreview. The card portals to <body>, so it is styled as a light popover
  regardless of the dark hero the chips sit on.
-->
<ul class="flex flex-wrap gap-[8px]">
  {#each judges as judge (judge.slug)}
    <li>
      <LinkPreview.Root openDelay={120} closeDelay={80}>
        <LinkPreview.Trigger
          class="flex cursor-default items-center gap-[8px] rounded-full bg-white/20 py-[4px] pl-[4px] pr-[12px] text-[14px] font-semibold text-white transition-colors hover:bg-white/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/60"
        >
          {#if judge.avatar_url}
            <img
              src={ikAvatar(judge.avatar_url, 24)}
              alt=""
              class="h-[24px] w-[24px] rounded-full object-cover"
            />
          {:else}
            <span
              class="flex h-[24px] w-[24px] items-center justify-center rounded-full bg-white/25 text-[12px] font-bold"
            >{initial(judge.name)}</span>
          {/if}
          {judge.name}
        </LinkPreview.Trigger>

        <LinkPreview.Content
          sideOffset={8}
          class="z-50 w-[300px] rounded-[10px] border border-brand-border bg-white p-4 text-left shadow-[0_12px_32px_rgba(15,23,42,0.18)]"
        >
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
        </LinkPreview.Content>
      </LinkPreview.Root>
    </li>
  {/each}
</ul>
