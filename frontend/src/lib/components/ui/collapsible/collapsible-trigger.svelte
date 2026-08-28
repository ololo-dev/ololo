<script lang="ts">
  import { Collapsible as CollapsiblePrimitive } from 'bits-ui';
  let { children, ...restProps }: { children?: import('svelte').Snippet; [key: string]: unknown } = $props();
</script>
<CollapsiblePrimitive.Trigger {...restProps}>
  {@render children?.()}
</CollapsiblePrimitive.Trigger>
