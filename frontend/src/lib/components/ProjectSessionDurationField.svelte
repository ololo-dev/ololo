<script lang="ts">
  interface Props {
    value?: string;
    onchange?: (value: string) => void;
    idleValue?: string;
    onIdleChange?: (value: string) => void;
    idPrefix?: string;
  }

  let { value = '', onchange, idleValue = '', onIdleChange, idPrefix = '' }: Props = $props();

  const pid = (name: string) => (idPrefix ? `${idPrefix}-${name}` : name);
</script>

<div class="mb-4 mt-6 border-t border-brand-border pt-4">
  <label for={pid('session-duration')} class="mb-1 block text-xs font-semibold text-brand-text">
    Session duration (secs)
    <span class="font-normal text-brand-muted">(60&ndash;86400, default 3600)</span>
  </label>
  <input
    id={pid('session-duration')}
    name="session_duration_secs"
    type="number"
    min="60"
    max="86400"
    placeholder="3600"
    {value}
    oninput={(e) => onchange?.((e.target as HTMLInputElement).value)}
    class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3
           text-base text-brand-text placeholder:text-brand-muted
           focus:border-brand-blue focus:outline-none"
  />

  <label for={pid('idle-timeout')} class="mb-1 mt-4 block text-xs font-semibold text-brand-text">
    Idle timeout (secs)
    <span class="font-normal text-brand-muted"
      >(cancel a running session after this long with no connected agents; 0 disables, default
      300)</span>
  </label>
  <input
    id={pid('idle-timeout')}
    name="idle_timeout_secs"
    type="number"
    min="0"
    max="86400"
    placeholder="300"
    value={idleValue}
    oninput={(e) => onIdleChange?.((e.target as HTMLInputElement).value)}
    class="h-[40px] w-full rounded-[8px] border-2 border-brand-border bg-white px-3
           text-base text-brand-text placeholder:text-brand-muted
           focus:border-brand-blue focus:outline-none"
  />
</div>
