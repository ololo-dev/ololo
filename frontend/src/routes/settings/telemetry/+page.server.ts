import { getLlmTelemetry } from "$lib/api";
import type { PageServerLoad } from "./$types";

export const load: PageServerLoad = async ({ fetch }) => {
  // First page, no filters. The endpoint may not be deployed yet during
  // rollout — degrade to an empty page rather than failing the whole tab.
  const telemetry = await getLlmTelemetry({}, { fetch }).catch(() => ({
    items: [],
    total: 0,
  }));
  return { telemetry };
};
