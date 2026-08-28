import { listJudges, listLlmProviders, listLlmPools, listJudgeUsage } from "$lib/api";
import type { PageServerLoad } from "./$types";

export const load: PageServerLoad = async ({ fetch }) => {
  const [judges, providers, pools, usage] = await Promise.all([
    listJudges({ fetch }),
    // Providers and pools power the optional per-judge override pickers.
    // Both degrade to empty so the page still edits judges if the LLM
    // registry is unavailable.
    listLlmProviders({ fetch }).catch(() => []),
    listLlmPools({ fetch }).catch(() => []),
    // Attachments and the judges' record. Degrades to empty: an admin who
    // came here to edit a prompt should not be stopped by an aggregate.
    listJudgeUsage({ fetch }).catch(() => []),
  ]);
  return { judges, providers, pools, usage };
};
