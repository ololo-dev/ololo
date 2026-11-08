<script lang="ts">
  import { page } from "$app/state";

  let { children } = $props();
</script>

<!-- One session per mount. The board flips between linked sessions with a
     client-side navigation to another code on this same route; the page
     component seeds its state from the first `data` it sees and dials one
     socket on mount, so the code is the key: a new code, a fresh page, the
     browser's fullscreen kept. -->
{#key page.params.code}
  {@render children()}
{/key}
