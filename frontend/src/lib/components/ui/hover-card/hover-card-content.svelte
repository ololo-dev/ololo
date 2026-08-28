<script lang="ts">
  import { LinkPreview as HoverCardPrimitive } from 'bits-ui';
  import { cn } from '$lib/utils';

  let {
    class: className = undefined,
    align = 'center',
    sideOffset = 6,
    children,
    ...restProps
  }: {
    class?: string;
    align?: 'start' | 'center' | 'end';
    sideOffset?: number;
    children?: import('svelte').Snippet;
    [key: string]: unknown;
  } = $props();
</script>

<HoverCardPrimitive.Content
  {align}
  {sideOffset}
  class={cn(
    'z-50 w-72 rounded-[8px] border border-brand-border bg-white p-3 shadow-lg outline-none',
    className,
  )}
  {...restProps}
>
  {@render children?.()}
</HoverCardPrimitive.Content>
