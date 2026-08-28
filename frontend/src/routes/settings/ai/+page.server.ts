import { listJudges, listLlmProviders, getLlmAssignments, listLlmPools } from "$lib/api";
import type { PageServerLoad } from "./$types";

export const load: PageServerLoad = async ({ fetch }) => {
  const [providers, assignments, judges, pools] = await Promise.all([
    listLlmProviders({ fetch }),
    getLlmAssignments({ fetch }),
    listJudges({ fetch }),
    listLlmPools({ fetch }),
  ]);
  return { providers, assignments, judges, pools };
};
