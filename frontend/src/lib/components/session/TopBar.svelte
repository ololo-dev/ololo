<script lang="ts">
  import type { Snippet } from "svelte";
  let {
    phase,
    participants,
    canControl,
    controlBusy,
    onControl,
    onCancel,
    projectName,
    children,
  }: {
    phase: "lobby" | "active" | "paused" | "finished";
    participants: { length: number };
    canControl: boolean;
    controlBusy: boolean;
    onControl: (next: "paused" | "running") => void;
    onCancel: () => void;
    projectName?: string | null;
    children: Snippet;
  } = $props();
</script>

<div
  class="flex items-center justify-between px-[24px] py-[14px]"
  style="background: #36547f;"
>
  <!-- Left: phase label + project name -->
  <div class="flex items-center gap-[12px]">
    {#if phase === "active"}
      <span
        class="inline-block h-[8px] w-[8px] shrink-0 rounded-full"
        style="background: #fb341c; animation: pulse 1.5s ease-in-out infinite;"
      ></span>
      <span class="text-[12px] font-semibold uppercase tracking-wider" style="color: rgba(255,255,255,0.85);">Live</span>
    {:else if phase === "paused"}
      <span class="text-[12px] font-semibold uppercase tracking-wider" style="color: rgba(255,255,255,0.85);">Paused</span>
    {:else if phase === "lobby"}
      <span class="text-[12px] font-semibold" style="color: rgba(255,255,255,0.7);">Starts in</span>
    {:else}
      <span class="text-[12px] font-semibold" style="color: rgba(255,255,255,0.7);">Session complete</span>
    {/if}
    {#if projectName}
      <span class="hidden text-[20px] font-bold text-white sm:inline" style="opacity: 0.9;">{projectName}</span>
    {/if}
  </div>

  <!-- Center: timer digits + transport controls (tape-recorder style) -->
  <div class="flex items-center gap-[12px]">
    <!-- Timer digits are rendered by the parent (snippet) since they depend on countdown state -->
    {@render children()}

    {#if canControl && (phase === "active" || phase === "paused")}
      <div class="ml-[8px] flex items-center gap-[6px]">
        {#if phase === "active"}
          <button
            type="button"
            onclick={() => onControl("paused")}
            disabled={controlBusy}
            title="Pause"
            aria-label="Pause session"
            class="flex h-[32px] w-[32px] items-center justify-center rounded-[6px] transition-all disabled:opacity-40"
            style="color: rgba(255,255,255,0.85); background: rgba(255,255,255,0.12);"
          >
            <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="5" width="4" height="14" rx="1"></rect><rect x="14" y="5" width="4" height="14" rx="1"></rect></svg>
          </button>
        {:else if phase === "paused"}
          <button
            type="button"
            onclick={() => onControl("running")}
            disabled={controlBusy}
            title="Resume"
            aria-label="Resume session"
            class="flex h-[32px] w-[32px] items-center justify-center rounded-[6px] transition-all disabled:opacity-40"
            style="color: rgba(255,255,255,0.95); background: rgba(107,229,151,0.25);"
          >
            <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><polygon points="6,4 20,12 6,20"></polygon></svg>
          </button>
        {/if}
        <button
          type="button"
          onclick={onCancel}
          disabled={controlBusy}
          title="Cancel"
          aria-label="Cancel session"
          class="flex h-[32px] w-[32px] items-center justify-center rounded-[6px] transition-all disabled:opacity-40"
          style="color: rgba(255,255,255,0.85); background: rgba(251,52,28,0.18);"
        >
          <svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><rect x="6" y="6" width="12" height="12" rx="1.5"></rect></svg>
        </button>
      </div>
    {/if}
  </div>

  <!-- Right: participant count -->
  <span class="text-[12px] font-semibold" style="color: rgba(255,255,255,0.55);">
    {#if phase === "active"}
      {participants.length} competing
    {:else}
      {participants.length} in lobby
    {/if}
  </span>
</div>
