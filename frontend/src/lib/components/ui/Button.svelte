<script lang="ts">
  import type { HTMLButtonAttributes, HTMLAnchorAttributes } from 'svelte/elements';

  type Variant = 'primary' | 'outline' | 'ghost' | 'danger';
  type Size = 'sm' | 'md' | 'lg' | 'full';

  interface Props extends HTMLButtonAttributes {
    variant?: Variant;
    size?: Size;
    href?: string;
  }

  let {
    variant = 'outline',
    size = 'md',
    href,
    class: className = '',
    children,
    ...rest
  }: Props = $props();

  const base =
    'inline-flex items-center justify-center font-heading font-bold rounded-btn border-2 transition-colors duration-300 text-base leading-normal';

  const variants: Record<Variant, string> = {
    primary: 'bg-brand-blue text-white border-brand-blue hover:bg-transparent hover:text-brand-blue',
    outline: 'bg-transparent text-brand-blue border-brand-blue hover:bg-brand-blue hover:text-white',
    ghost: 'bg-transparent border-transparent text-brand-text hover:text-brand-blue',
    danger: 'bg-transparent text-brand-red border-brand-red hover:bg-brand-red hover:text-white',
  };

  const sizes: Record<Size, string> = {
    sm: 'px-4 py-2 min-w-[100px] text-sm',
    md: 'px-5 py-[10px] min-w-[120px]',
    lg: 'px-6 py-6 min-w-[170px]',
    full: 'w-full px-5 py-[10px]',
  };

  const classes = $derived([base, variants[variant], sizes[size], className].filter(Boolean).join(' '));
</script>

{#if href}
  <a {href} class={classes} {...(rest as unknown as HTMLAnchorAttributes)}>
    {@render children?.()}
  </a>
{:else}
  <button class={classes} {...rest}>
    {@render children?.()}
  </button>
{/if}
