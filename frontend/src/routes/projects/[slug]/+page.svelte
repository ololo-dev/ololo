<script lang="ts">
  import { ikCover } from '$lib/imagekit';
  import type { PageData } from "./$types";
  import ProjectDetailPage from "$lib/components/ProjectDetailPage.svelte";
  import { SITE_URL, pageTitle, DEFAULT_DESCRIPTION } from "$lib/seo";

  let { data }: { data: PageData } = $props();

  const metaDescription = $derived(
    data.project.description?.trim()
      ? data.project.description.trim().slice(0, 160)
      : DEFAULT_DESCRIPTION,
  );
</script>

<svelte:head>
  <title>{pageTitle(data.project.name)}</title>
  <meta name="description" content={metaDescription} />
  {#if data.project.slug}
    <link rel="canonical" href="{SITE_URL}/projects/{data.project.slug}" />
  {/if}
  <meta property="og:title" content={pageTitle(data.project.name)} />
  <meta property="og:description" content={metaDescription} />
  {#if data.project.cover_image_url}
    <meta property="og:image" content={ikCover(data.project.cover_image_url, 1200, 630)} />
  {/if}
</svelte:head>

<ProjectDetailPage
  project={data.project}
  taskPreview={data.taskPreview}
  parts={data.parts}
  ssrSessions={data.sessions}
  judges={data.judges}
  topPlayers={data.topPlayers}
  message={data.message}
  currentUserId={data.currentUserId}
  isAdmin={data.isAdmin}
/>
