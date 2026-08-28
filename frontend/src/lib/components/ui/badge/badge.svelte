<script lang="ts">
  import { cn } from '$lib/utils';
  let { class: className = '', variant = 'default', ...restProps }: { class?: string; variant?: 'default' | 'outline'; [key: string]: unknown } = $props();
  const variants: Record<string, string> = {
    default: 'bg-slate-900 text-white',
    outline: 'border border-slate-300 text-slate-700',
  };
</script>
<span class={cn('inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-semibold', variants[variant], className)} {...restProps}></span>
