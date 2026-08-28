// Cached model-suggestion loader for configured LLM providers.
//
// Merges the live provider models endpoint with the models.dev catalog
// (when the provider is linked to a catalog entry). Results are cached in a
// module-level map so repeated pickers share one fetch per provider.

import { listLlmProviderModels, type LlmProvider } from "$lib/api";
import { loadModelsDevCatalog } from "./models-dev";

const cache = new Map<string, Promise<string[]>>();

/** Merged, cached model id suggestions for one provider. Never rejects —
 * an unreachable provider or catalog just yields fewer (possibly zero) ids. */
export function modelSuggestions(provider: LlmProvider): Promise<string[]> {
  const key = `${provider.id}:${provider.catalog_id ?? ""}`;
  let promise = cache.get(key);
  if (!promise) {
    promise = fetchMerged(provider);
    cache.set(key, promise);
  }
  return promise;
}

async function fetchMerged(provider: LlmProvider): Promise<string[]> {
  let merged: string[] = [];
  try {
    merged = await listLlmProviderModels(provider.id);
  } catch {
    // Unreachable provider — free-form entry still works.
  }
  if (provider.catalog_id) {
    try {
      const catalog = await loadModelsDevCatalog();
      const entry = catalog.find((c) => c.id === provider.catalog_id);
      if (entry) merged = [...new Set([...merged, ...entry.models])];
    } catch {
      // Catalog unavailable — live models only.
    }
  }
  return [...merged].sort();
}
