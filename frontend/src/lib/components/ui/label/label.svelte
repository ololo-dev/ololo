<script lang="ts">
  import { cn } from '$lib/utils';
  let { class: className = '', ...restProps }: { class?: string; [key: string]: unknown } = $props();
</script>
<label class={cn('text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70', className)} {...restProps}></label>
