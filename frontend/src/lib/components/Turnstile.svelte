<script lang="ts">
  import { browser } from "$app/environment";
  import { onMount } from "svelte";

  interface Props {
    sitekey: string;
    ontoken: (token: string) => void;
  }

  let { sitekey, ontoken }: Props = $props();

  let container: HTMLDivElement | null = $state(null);
  let widgetId: string | null = null;

  // Turnstile tokens are single-use: after a rejected submit the parent must
  // reset the widget to obtain a fresh token, or the next submit goes out
  // with an empty/stale one and fails captcha validation again.
  export function reset() {
    ontoken("");
    if (widgetId !== null && typeof window !== "undefined" && (window as any).turnstile) {
      (window as any).turnstile.reset(widgetId);
    }
  }

  onMount(() => {
    let cancelled = false;

    const loadScript = (): Promise<void> => {
      return new Promise((resolve, reject) => {
        if (typeof window !== "undefined" && (window as any).turnstile) {
          resolve();
          return;
        }
        const existing = document.querySelector(
          'script[src*="challenges.cloudflare.com/turnstile"]',
        );
        if (existing) {
          existing.addEventListener("load", () => resolve());
          existing.addEventListener("error", () => reject(new Error("script load error")));
          return;
        }
        const script = document.createElement("script");
        script.src = "https://challenges.cloudflare.com/turnstile/v0/api.js";
        script.async = true;
        script.defer = true;
        script.onload = () => resolve();
        script.onerror = () => reject(new Error("failed to load turnstile script"));
        document.head.appendChild(script);
      });
    };

    const renderWidget = async () => {
      try {
        await loadScript();
        if (cancelled || !container) return;
        const ts = (window as any).turnstile;
        if (!ts) return;
        const existing = ts.getWidgetId ? ts.getWidgetId(container) : widgetId;
        if (existing) ts.remove(existing);
        widgetId = ts.render(container, {
          sitekey,
          callback: (token: string) => ontoken(token),
          "refresh-expired": "auto",
          "expired-callback": () => ontoken(""),
          "error-callback": () => ontoken(""),
        });
      } catch (e) {
        console.error("Turnstile load failed:", e);
      }
    };

    renderWidget();

    return () => {
      cancelled = true;
      if (widgetId && typeof window !== "undefined" && (window as any).turnstile) {
        (window as any).turnstile.remove(widgetId);
      }
    };
  });
</script>

{#if browser}
  <div bind:this={container} class="turnstile-widget"></div>
{/if}