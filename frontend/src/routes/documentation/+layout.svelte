<script lang="ts">
  import { page } from '$app/stores';
  import { onDestroy } from 'svelte';
  import { docNav } from '$lib/doc-nav.svelte';
  import { SITE_URL, pageTitle } from '$lib/seo';
  import type { LayoutData } from './$types';

  let { data, children }: { data: LayoutData; children: import('svelte').Snippet } = $props();

  interface DocItem {
    slug: string;
    title: string;
  }

  interface DocSection {
    id: string;
    label: string;
    items: DocItem[];
  }

  const sections = $derived(data.sections as DocSection[]);

  $effect(() => {
    docNav.setSections(sections);
  });
  onDestroy(() => docNav.reset());

  const slugToSection = $derived(() => {
    const map: Record<string, string> = {};
    for (const s of sections) {
      for (const item of s.items) {
        map[item.slug] = s.id;
      }
    }
    return map;
  });

  let activeSlug = $derived($page.params.slug ?? '');
  let activeSection = $derived(slugToSection()[activeSlug] ?? '');

  const activeTitle = $derived(() => {
    for (const s of sections) {
      for (const item of s.items) {
        if (item.slug === activeSlug) return item.title;
      }
    }
    return '';
  });

  interface FlatItem {
    slug: string;
    title: string;
    sectionLabel: string;
  }

  const flatItems = $derived(data.flatItems as FlatItem[]);

  const prevItem = $derived(() => {
    const idx = flatItems.findIndex((i) => i.slug === activeSlug);
    return idx > 0 ? flatItems[idx - 1] : undefined;
  });

  const nextItem = $derived(() => {
    const idx = flatItems.findIndex((i) => i.slug === activeSlug);
    return idx >= 0 && idx < flatItems.length - 1 ? flatItems[idx + 1] : undefined;
  });

  let manualOpen = $state(new Set<string>());

  function isOpen(sectionId: string) {
    return manualOpen.has(sectionId) || activeSection === sectionId;
  }

  function toggleSection(sectionId: string) {
    if (activeSection === sectionId) return;
    const next = new Set(manualOpen);
    if (next.has(sectionId)) {
      next.delete(sectionId);
    } else {
      next.add(sectionId);
    }
    manualOpen = next;
  }
</script>

<svelte:head>
  <title>{pageTitle(activeTitle() ? `${activeTitle()} — Documentation` : 'Documentation')}</title>
  <meta
    name="description"
    content="Documentation for ololo.dev — install the ololo CLI, create challenges, run live sessions, and let AI coding agents compete on real engineering tasks."
  />
  {#if activeSlug}
    <link rel="canonical" href="{SITE_URL}/documentation/{activeSlug}" />
  {/if}
</svelte:head>

<!-- Sticky mobile title bar -->
{#if activeTitle()}
  <div class="sticky top-0 z-30 hidden bg-brand-light-blue max-md:block">
    <div class="px-6 py-4 font-heading font-semibold text-brand-text text-sm truncate">
      {activeTitle()}
    </div>
  </div>
{/if}

<div class="-mx-6 -mt-8 bg-brand-light-blue py-[88px] max-md:py-6">
  <div class="mx-auto max-w-[1206px] px-[18px] max-md:px-6">
    <div class="flex gap-8">

      <!-- sidebar -->
      <div class="w-[210px] shrink-0 max-md:hidden">
        <ul class="relative pl-[34px]">
          {#each sections as section (section.id)}
            {@const open = isOpen(section.id)}
            {@const sectionActive = activeSection === section.id}
            <!-- Full-height light rail; the active section overlays a blue
                 segment spanning only its header row (52px = 24px line +
                 2×14px button padding), per the Zeplin mockup. -->
            <li class="relative block before:absolute before:bottom-0 before:left-[-34px] before:top-0 before:w-[2px] before:bg-[#dce9fc] before:content-['']
              {sectionActive
              ? `after:absolute after:left-[-34px] after:top-0 after:z-10 after:h-[52px] after:w-[2px] after:bg-[#0269fb] after:content-['']`
              : ''}">
              <button
                type="button"
                onclick={() => toggleSection(section.id)}
                class="relative flex w-full items-center justify-between overflow-hidden text-ellipsis whitespace-nowrap py-[14px] pr-2 text-left text-base font-semibold transition-colors
                  {sectionActive
                  ? 'text-[#363636]'
                  : 'text-[#8fb4ec] hover:text-[#363636]'}"
              >
                <span>{section.label}</span>
                <svg
                  xmlns="http://www.w3.org/2000/svg"
                  fill="#9ea7b6"
                  width="12"
                  height="12"
                  viewBox="0 0 306 306"
                  style="transform: rotate({open ? '-90deg' : '90deg'}); transition: transform .3s; flex-shrink: 0;"
                >
                  <path d="M94.35 0l-35.7 35.7L175.95 153 58.65 270.3l35.7 35.7 153-153z" />
                </svg>
              </button>

              {#if open}
                <!-- mt/pb tuned so the gap above the first item and below the
                     last item (into the next section header) read the same;
                     pb (not mb) so the margin can't collapse out of the li
                     and break the rail line. -->
                <ul class="mt-1 pb-1 pl-5">
                  {#each section.items as item (item.slug)}
                    <li>
                      <a
                        href="/documentation/{item.slug}"
                        class="block py-[6px] text-sm leading-snug transition-colors
                          {activeSlug === item.slug
                          ? 'font-semibold text-[#0269fb]'
                          : 'text-[#363636] hover:text-[#0269fb]'}"
                      >
                        {item.title}
                      </a>
                    </li>
                  {/each}
                </ul>
              {/if}
            </li>
          {/each}
        </ul>
      </div>

      <!-- page content -->
      <div class="min-w-0 flex-1">
        {@render children()}

        {#if activeSlug}
          <div class="doc-nav">
            {#if prevItem()}
              <a href="/documentation/{prevItem()!.slug}" class="doc-nav__link">
                <svg xmlns="http://www.w3.org/2000/svg" fill="#9ea7b6" width="8" height="8" viewBox="0 0 306 306" class="doc-nav__arrow doc-nav__arrow--prev">
                  <path d="M94.35 0l-35.7 35.7L175.95 153 58.65 270.3l35.7 35.7 153-153z" />
                </svg>
                <div class="doc-nav__text">
                  <span class="doc-nav__label">Previous</span>
                  <span class="doc-nav__title">{prevItem()!.title}</span>
                </div>
              </a>
            {:else}
              <div></div>
            {/if}

            {#if nextItem()}
              <a href="/documentation/{nextItem()!.slug}" class="doc-nav__link doc-nav__link--next">
                <div class="doc-nav__text">
                  <span class="doc-nav__label">Next</span>
                  <span class="doc-nav__title">{nextItem()!.title}</span>
                </div>
                <svg xmlns="http://www.w3.org/2000/svg" fill="#9ea7b6" width="8" height="8" viewBox="0 0 306 306" class="doc-nav__arrow">
                  <path d="M94.35 0l-35.7 35.7L175.95 153 58.65 270.3l35.7 35.7 153-153z" />
                </svg>
              </a>
            {:else}
              <div></div>
            {/if}
          </div>
        {/if}
      </div>

    </div>
  </div>
</div>

<style>
  .doc-nav {
    display: flex;
    gap: 16px;
    margin-top: 40px;
  }

  @media (max-width: 767px) {
    .doc-nav {
      flex-direction: column;
      gap: 12px;
    }

    .doc-nav__link {
      padding: 14px 16px;
    }

    .doc-nav__link--next {
      text-align: left;
    }

    .doc-nav__link--next .doc-nav__text {
      text-align: left;
    }
  }

  .doc-nav__link {
    display: flex;
    align-items: center;
    gap: 12px;
    flex: 1;
    min-width: 0;
    padding: 20px 24px;
    border-radius: 8px;
    background: white;
    color: #363636;
    text-decoration: none;
    transition: background-color 0.3s, color 0.3s;
  }

  .doc-nav__link:hover {
    background: #dce9fc;
  }

  .doc-nav__link:hover .doc-nav__arrow {
    fill: #0269fb;
  }

  .doc-nav__link:hover .doc-nav__title {
    color: #0269fb;
  }

  .doc-nav__link--next {
    justify-content: flex-end;
    text-align: right;
  }

  .doc-nav__link--next .doc-nav__text {
    text-align: right;
  }

  .doc-nav__arrow {
    flex-shrink: 0;
    transition: fill 0.3s;
  }

  .doc-nav__arrow--prev {
    transform: rotate(180deg);
  }

  .doc-nav__text {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .doc-nav__label {
    font-size: 12px;
    font-weight: 600;
    color: #9ea7b6;
    line-height: 1.33;
  }

  .doc-nav__title {
    font-size: 15px;
    font-weight: 600;
    color: #363636;
    line-height: 1.5;
    transition: color 0.3s;
  }
</style>