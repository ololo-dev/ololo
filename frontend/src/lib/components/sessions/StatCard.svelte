<script lang="ts">
  let {
    label,
    value,
    valueClass = "text-brand-text",
    minWidth = true,
  }: {
    label: string;
    value: string;
    valueClass?: string;
    minWidth?: boolean;
  } = $props();
</script>

<div
  class="rounded-[8px] bg-white px-5 py-3 text-center shadow-sm {minWidth ? 'min-w-[88px]' : ''}"
>
  <div class="mb-0.5 text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
    {label}
  </div>
  <div class="font-heading text-[26px] font-bold {valueClass}">{value}</div>
</div>
