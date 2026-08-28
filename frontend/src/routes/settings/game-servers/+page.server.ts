import type { PageServerLoad } from "./$types";

export const load: PageServerLoad = async () => {
  // Data loaded client-side via getGameServers() in +page.svelte.
  return {};
};
