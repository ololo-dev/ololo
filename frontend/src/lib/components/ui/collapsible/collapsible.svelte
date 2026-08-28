<script lang="ts">
  import { Collapsible as CollapsiblePrimitive } from 'bits-ui';
  let { open = $bindable(true), onOpenChange, children, ...restProps }: {
    open?: boolean;
    onOpenChange?: (open: boolean) => void;
    children?: import('svelte').Snippet;
    [key: string]: unknown;
  } = $props();
</script>
<CollapsiblePrimitive.Root
  open={open}
  onOpenChange={(v) => { open = v; onOpenChange?.(v); }}
  {...restProps}
>
  {@render children?.()}
</CollapsiblePrimitive.Root>
