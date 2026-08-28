import { getCostsSummary, getSettings } from "$lib/api";
import type { PageServerLoad } from "./$types";

export const load: PageServerLoad = async ({ fetch }) => {
  // Degrade to an empty page during rollout rather than failing the tab.
  const [summary, settings] = await Promise.all([
    getCostsSummary(30, { fetch }).catch(() => null),
    getSettings({ fetch }).catch(() => ({}) as Record<string, string>),
  ]);
  return {
    summary,
    showCostsInSession:
      (settings as Record<string, string>)["show_llm_costs_in_session"] === "true",
  };
};
