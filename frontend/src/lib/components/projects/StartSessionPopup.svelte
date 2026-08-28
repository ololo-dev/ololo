<script lang="ts">
  import CodeBlock from "$lib/components/CodeBlock.svelte";

  let {
    slug,
    open = $bindable(),
  }: {
    slug: string;
    open: boolean;
  } = $props();
</script>

{#if open && slug}
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
    role="dialog"
    aria-modal="true"
    aria-label="Start session"
  >
    <div
      class="popup-card relative w-full max-w-[520px] rounded-[8px] bg-white px-6 py-8 sm:px-[48px] sm:py-[40px]"
    >
      <!-- Close button -->
      <button
        type="button"
        onclick={() => (open = false)}
        aria-label="Close"
        class="absolute right-[20px] top-[20px] flex h-[32px] w-[32px] items-center justify-center rounded-full text-brand-muted transition-opacity hover:opacity-60"
      >
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="20"
          height="20"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2.5"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <line x1="18" y1="6" x2="6" y2="18" /><line
            x1="6"
            y1="6"
            x2="18"
            y2="18"
          />
        </svg>
      </button>

      <h3
        class="mb-[8px] font-heading text-[26px] font-bold leading-[1.23] text-brand-text"
      >
        Start session
      </h3>
      <p class="mb-[24px] text-[15px] text-brand-muted">
        Enter this command in your console to start the session.
      </p>

      <CodeBlock code="ololo start {slug}" />
    </div>
  </div>
{/if}