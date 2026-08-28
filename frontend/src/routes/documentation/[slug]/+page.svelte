<script lang="ts">
  import { browser } from '$app/environment';
  import type { PageData } from './$types';

  let { data }: { data: PageData } = $props();

  // mdsvex component — must be capitalised so Svelte treats it as a component.
  const Doc = $derived(data.component);
  const title = $derived((data.metadata?.title as string) ?? '');
  const section = $derived((data.metadata?.section as string) ?? '');

  let article = $state<HTMLElement | undefined>();

  const ICON_COPY = `<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="9" y="2" width="6" height="4" rx="1"/><path d="M8 4H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V6a2 2 0 0 0-2-2h-2"/></svg>`;
  const ICON_CHECK = `<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="20 6 9 17 4 12"/></svg>`;

  $effect(() => {
    // Track the mdsvex component so this re-runs on doc page changes.
    void data.component;

    if (!browser || !article) return;

    const buttons: HTMLButtonElement[] = [];
    const timers: ReturnType<typeof setTimeout>[] = [];

    article.querySelectorAll<HTMLPreElement>('pre').forEach((pre) => {
      const code = pre.querySelector('code')?.textContent ?? pre.textContent ?? '';

      const btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'copy-btn';
      btn.setAttribute('aria-label', 'Copy to clipboard');
      btn.setAttribute('title', 'Copy to clipboard');
      btn.innerHTML = ICON_COPY;

      btn.addEventListener('click', async () => {
        try {
          await navigator.clipboard.writeText(code);
          btn.innerHTML = ICON_CHECK;
          btn.setAttribute('aria-label', 'Copied!');
          btn.setAttribute('title', 'Copied!');
          const t = setTimeout(() => {
            btn.innerHTML = ICON_COPY;
            btn.setAttribute('aria-label', 'Copy to clipboard');
            btn.setAttribute('title', 'Copy to clipboard');
          }, 1500);
          timers.push(t);
        } catch {
          // clipboard unavailable
        }
      });

      pre.appendChild(btn);
      buttons.push(btn);
    });

    return () => {
      timers.forEach(clearTimeout);
      buttons.forEach((b) => b.remove());
    };
  });
</script>

<article bind:this={article}>
  {#if title}
    <h2>{title}</h2>
  {/if}

  <ul class="breadcrumbs">
    <li><a href="/documentation">Documentation</a></li>
    {#if section}<li><span>{section}</span></li>{/if}
    {#if title}<li><span>{title}</span></li>{/if}
  </ul>

  <Doc />
</article>

<style>
  article {
    background: white;
    border-radius: 8px;
    padding: 48px 88px 56px;
    min-height: 600px;
  }

  /* ── Breadcrumbs ─────────────────────────────────────────────────────── */
  .breadcrumbs {
    display: flex;
    flex-wrap: wrap;
    font-size: 16px;
    font-weight: 700;
    line-height: 1.5;
    color: #9ea7b6;
    margin-bottom: 24px;
    list-style: none;
    padding: 0;
  }

  .breadcrumbs li {
    position: relative;
    display: block;
  }

  /* Chevron separator between items */
  .breadcrumbs li:not(:last-child) {
    padding-right: 32px;
  }

  .breadcrumbs li:not(:last-child)::before {
    position: absolute;
    content: '';
    top: 5px;
    right: 12px;
    width: 12px;
    height: 12px;
    background-size: contain;
    background-repeat: no-repeat;
    background-position: center;
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='306' height='306' viewBox='0 0 306 306'%3E%3Cpath fill='%239ea7b6' d='M94.35 0l-35.7 35.7L175.95 153 58.65 270.3l35.7 35.7 153-153z'/%3E%3C/svg%3E");
  }

  .breadcrumbs a {
    color: #9ea7b6;
    text-decoration: none;
    transition: opacity 0.3s;
  }

  .breadcrumbs a:hover {
    opacity: 0.7;
  }

  /* ── Article headings ────────────────────────────────────────────────── */
  article :global(h2) {
    font-size: 40px;
    font-weight: 700;
    color: #363636;
    margin-bottom: 24px;
    line-height: 1.2;
  }

  article :global(h3) {
    font-size: 34px;
    font-weight: 700;
    color: #363636;
    margin-top: 40px;
    margin-bottom: 20px;
    line-height: 1.18;
  }

  article :global(h4) {
    font-size: 26px;
    font-weight: 700;
    color: #363636;
    margin-top: 40px;
    margin-bottom: 16px;
    line-height: 1.23;
  }

  article :global(h5) {
    font-size: 18px;
    font-weight: 700;
    color: #363636;
    margin-top: 40px;
    margin-bottom: 16px;
    line-height: 1.33;
  }

  article :global(p) {
    font-size: 15px;
    color: #363636;
    line-height: 1.7;
    margin-top: 16px;
    margin-bottom: 16px;
  }

  article :global(a) {
    color: #0269fb;
    text-decoration: underline;
  }

  article :global(blockquote) {
    background: #dce9fc;
    color: #36547f;
    font-style: italic;
    font-weight: 500;
    line-height: 1.5;
    padding: 16px 32px;
    margin-top: 40px;
    margin-bottom: 40px;
    border-radius: 8px;
    display: block;
  }

  article :global(blockquote p) {
    color: #36547f;
    margin: 0;
  }

  /* ── Tables ──────────────────────────────────────────────────────────── */
  article :global(table) {
    width: 100%;
    border-collapse: separate;
    border-spacing: 0;
    margin-top: 24px;
    margin-bottom: 24px;
    font-size: 15px;
    line-height: 1.5;
    border: 1px solid #dce9fc;
    border-radius: 8px;
  }

  article :global(th) {
    background: #eef4fd;
    color: #36547f;
    font-size: 14px;
    font-weight: 600;
    text-align: left;
    padding: 12px 20px;
  }

  article :global(th:first-child) {
    border-top-left-radius: 7px;
  }

  article :global(th:last-child) {
    border-top-right-radius: 7px;
  }

  article :global(td) {
    padding: 12px 20px;
    color: #363636;
    border-top: 1px solid #eef4fd;
    vertical-align: top;
  }

  article :global(tbody tr:last-child td:first-child) {
    border-bottom-left-radius: 7px;
  }

  article :global(tbody tr:last-child td:last-child) {
    border-bottom-right-radius: 7px;
  }

  article :global(pre) {
    position: relative;
    background: #36547f;
    border-radius: 8px;
    padding: 20px 52px 20px 40px;
    overflow-x: auto;
    margin-top: 16px;
    margin-bottom: 16px;
    display: block;
  }

  /* Red left bar via pseudo-element on pre */
  article :global(pre::before) {
    position: absolute;
    content: '';
    top: 0;
    left: 0;
    bottom: 0;
    width: 8px;
    background-color: #fb341c;
    border-radius: 8px 0 0 8px;
  }

  article :global(pre code) {
    color: #fff;
    font-family: 'Roboto Mono', 'Fira Mono', 'Consolas', monospace;
    font-size: 14px;
    line-height: 1.5;
    background: transparent;
    padding: 0;
    white-space: pre;
  }

  article :global(code) {
    font-family: 'Roboto Mono', 'Fira Mono', 'Consolas', monospace;
    background: #dce9fc;
    color: #36547f;
    padding: 2px 6px;
    border-radius: 3px;
    font-size: 13px;
  }

  /* ── Lists ───────────────────────────────────────────────────────────── */
  /* ul without a class → Pakipaki bullet style */
  article :global(ul:not([class])) {
    margin-top: 16px;
    margin-bottom: 40px;
  }

  article :global(ul:not([class]) li) {
    position: relative;
    padding-left: 36px;
    display: block;
    font-size: 15px;
    color: #363636;
    line-height: 1.7;
  }

  article :global(ul:not([class]) li::before) {
    position: absolute;
    content: '';
    top: 9px;
    left: 13px;
    width: 4px;
    height: 4px;
    border-radius: 50%;
    background-color: #0269fb;
  }

  /* ol without a class → Pakipaki counter style */
  article :global(ol:not([class])) {
    counter-reset: list;
    margin-top: 16px;
    margin-bottom: 40px;
  }

  article :global(ol:not([class]) li) {
    position: relative;
    padding-left: 36px;
    display: block;
    font-size: 15px;
    color: #363636;
    line-height: 1.7;
  }

  article :global(ol:not([class]) li::before) {
    position: absolute;
    counter-increment: list;
    content: counter(list) '.';
    top: 0;
    left: 13px;
    color: #363636;
    font-weight: 700;
  }

  /* ── Misc ────────────────────────────────────────────────────────────── */
  article :global(img) {
    position: relative;
    border-radius: 8px;
    overflow: hidden;
    max-width: 100%;
    display: block;
    margin-top: 16px;
    margin-bottom: 16px;
  }

  article :global(figcaption) {
    margin-top: 16px;
    color: #9ea7b6;
    font-size: 12px;
    font-weight: 600;
    line-height: 1.33;
  }

  article :global(hr) {
    border: none;
    border-top: 1px solid #e7eaee;
    margin: 40px 0;
  }

  /* ── Code block copy button ──────────────────────────────────────────── */
  article :global(pre .copy-btn) {
    position: absolute;
    top: 12px;
    right: 12px;
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.1);
    color: rgba(255, 255, 255, 0.6);
    border: none;
    cursor: pointer;
    padding: 0;
    transition:
      background 0.15s,
      color 0.15s;
  }

  article :global(pre .copy-btn:hover) {
    background: rgba(255, 255, 255, 0.2);
    color: white;
  }

  /* ── Responsive: mobile ──────────────────────────────────────────────── */
  @media (max-width: 767px) {
    article {
      padding: 24px 20px 32px;
    }

    .breadcrumbs {
      font-size: 14px;
    }

    .breadcrumbs li:not(:last-child) {
      padding-right: 24px;
    }

    article :global(h2) {
      font-size: 28px;
      margin-bottom: 16px;
    }

    article :global(h3) {
      font-size: 24px;
      margin-top: 28px;
      margin-bottom: 16px;
    }

    article :global(h4) {
      font-size: 20px;
      margin-top: 24px;
      margin-bottom: 12px;
    }

    article :global(h5) {
      font-size: 16px;
      margin-top: 24px;
      margin-bottom: 12px;
    }

    article :global(blockquote) {
      padding: 12px 20px;
    }

    article :global(pre) {
      padding: 16px;
      border-radius: 6px;
    }

    article :global(pre::before) {
      width: 6px;
      border-radius: 6px 0 0 6px;
    }

    article :global(ul:not([class]) li),
    article :global(ol:not([class]) li) {
      padding-left: 24px;
    }

    article :global(ul:not([class]) li::before) {
      left: 8px;
    }

    article :global(ol:not([class]) li::before) {
      left: 8px;
    }

    article :global(pre .copy-btn) {
      top: 8px;
      right: 8px;
      width: 24px;
      height: 24px;
    }

    /* Narrow screens: let wide tables scroll inside the article instead
       of stretching the page. */
    article :global(table) {
      display: block;
      overflow-x: auto;
    }

    article :global(th),
    article :global(td) {
      padding: 10px 14px;
      white-space: nowrap;
    }
  }
</style>
