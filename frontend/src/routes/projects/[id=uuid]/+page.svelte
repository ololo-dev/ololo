<script lang="ts">
  import type { PageData } from "./$types";
  import ProjectDetailPage from "$lib/components/ProjectDetailPage.svelte";

  let { data }: { data: PageData } = $props();
</script>

<svelte:head>
  <title>{data.project.name} — ololo.dev</title>
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
