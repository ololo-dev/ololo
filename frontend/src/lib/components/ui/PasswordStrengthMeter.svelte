<script lang="ts">
  import { scorePassword } from '$lib/password-strength';

  interface Props {
    password: string;
    /** Wired to the field via aria-describedby so the label is announced. */
    id?: string;
  }

  let { password, id }: Props = $props();

  const strength = $derived(scorePassword(password));

  // Brand tokens at both ends, so the meter belongs to this form: the same
  // `brand-red` as the field's own validation error, and the same
  // `brand-blue` as the Register button right below it — one token, so they
  // cannot drift apart. Amber only for the middle step, which the site
  // already uses for "not there yet" (the judge score bars).
  // The label carries the meaning regardless: colour is never the only signal.
  const barColour = $derived(
    strength.score <= 1 ? 'bg-brand-red' : strength.score === 2 ? 'bg-amber-500' : 'bg-brand-blue'
  );
  const textColour = $derived(
    strength.score <= 1
      ? 'text-brand-red'
      : strength.score === 2
        ? 'text-amber-600'
        : 'text-brand-blue'
  );
</script>

{#if password.length > 0}
  <div class="mt-1.5" data-testid="password-strength">
    <div class="flex items-center gap-2">
      <div class="flex h-1 flex-1 gap-1" aria-hidden="true">
        {#each [1, 2, 3, 4] as segment (segment)}
          <div
            class="h-full flex-1 rounded-full transition-colors {segment <= strength.score
              ? barColour
              : 'bg-brand-border'}"
          ></div>
        {/each}
      </div>
      <span
        class="shrink-0 text-xs font-semibold leading-none {textColour}"
        data-testid="password-strength-label">{strength.label}</span
      >
    </div>
    <!-- Announced on change rather than on every keystroke: `polite` waits
         for a pause, which is what makes this bearable in a screen reader. -->
    <p
      {id}
      aria-live="polite"
      class="mt-1 block min-h-[1rem] text-xs leading-none text-brand-muted"
      data-testid="password-strength-hint"
    >
      {strength.hint}
    </p>
  </div>
{/if}
