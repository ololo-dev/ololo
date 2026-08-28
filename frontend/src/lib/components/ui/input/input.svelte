<script lang="ts">
  import { cn } from '$lib/utils';
  let { class: className = '', ...restProps }: { class?: string; [key: string]: unknown } = $props();
</script>
<input class={cn('flex h-10 w-full rounded-md border border-slate-300 bg-transparent px-3 py-2 text-sm placeholder:text-slate-400 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate-900 disabled:cursor-not-allowed disabled:opacity-50', className)} {...restProps} />
