// Lazy, module-cached loader for the models.dev provider catalog.
//
// The catalog (https://models.dev/api.json) is ~3.3 MB, so it is fetched at
// most once per page lifetime and the parsed result is cached in a
// module-level variable. A failed fetch clears the cache so a later attempt
// can retry; callers must always allow manual entry as a fallback.

export interface ModelsDevProvider {
  id: string;
  name: string;
  /**
   * Provider API base URL. From the catalog's own `api` field where it has
   * one, otherwise from {@link KNOWN_BASE_URLS}; null when neither knows it.
   */
  api: string | null;
  /** True when `api` came from our list rather than from the catalog. */
  apiFromFallback: boolean;
  /** Provider documentation, for endpoints nobody could fill in for you. */
  doc: string | null;
  /** Model ids known to the catalog for this provider. */
  models: string[];
}

/**
 * OpenAI-compatible base URLs for providers the catalog leaves blank.
 *
 * models.dev only records `api` for providers whose endpoint its SDK does not
 * already hardcode — which means it is missing for most of the ones you would
 * reach for first (OpenAI, Anthropic, Groq, Cerebras, Mistral, xAI…). Without
 * this list, picking any of them from the catalog leaves Base URL empty and
 * the form rejects the save.
 *
 * Every entry was verified by requesting `{base}/models` and getting an
 * auth-ish response (401/403) or a model list (200) rather than a 404.
 * Providers whose endpoint is per-resource (Azure, Bedrock, Vertex) are
 * deliberately absent — there is no single URL to suggest. Perplexity,
 * Google and v0 are absent because the paths I tried 404'd; better to leave
 * the field blank than to prefill a wrong value.
 */
const KNOWN_BASE_URLS: Record<string, string> = {
  openai: "https://api.openai.com/v1",
  anthropic: "https://api.anthropic.com/v1",
  groq: "https://api.groq.com/openai/v1",
  cerebras: "https://api.cerebras.ai/v1",
  mistral: "https://api.mistral.ai/v1",
  xai: "https://api.x.ai/v1",
  togetherai: "https://api.together.xyz/v1",
  deepinfra: "https://api.deepinfra.com/v1/openai",
  cohere: "https://api.cohere.ai/compatibility/v1",
  vercel: "https://ai-gateway.vercel.sh/v1",
  venice: "https://api.venice.ai/api/v1",
  aihubmix: "https://aihubmix.com/v1",
};

/**
 * Which logo id a configured provider should render with.
 *
 * Normally the catalog id it was linked to. An Ollama daemon is the common
 * case that has none — it is a local endpoint, not a hosted service anyone
 * would pick from the catalog — so fall back to its kind, which ProviderIcon
 * draws from its own built-in marks. Without this, every Ollama row falls
 * through to the bare letter badge.
 */
export function providerLogoId(p: { catalog_id: string | null; kind: string }): string | null {
  return p.catalog_id ?? (p.kind === "ollama" ? "ollama" : null);
}

let catalogPromise: Promise<ModelsDevProvider[]> | null = null;

export function loadModelsDevCatalog(): Promise<ModelsDevProvider[]> {
  if (!catalogPromise) {
    catalogPromise = fetchCatalog().catch((err) => {
      catalogPromise = null;
      throw err;
    });
  }
  return catalogPromise;
}

async function fetchCatalog(): Promise<ModelsDevProvider[]> {
  const resp = await fetch("https://models.dev/api.json");
  if (!resp.ok) {
    throw new Error(`models.dev catalog fetch failed: ${resp.status}`);
  }
  const data = (await resp.json()) as Record<string, unknown>;
  const providers: ModelsDevProvider[] = [];
  for (const [id, raw] of Object.entries(data)) {
    if (!raw || typeof raw !== "object") continue;
    const entry = raw as { name?: unknown; api?: unknown; doc?: unknown; models?: unknown };
    const models =
      entry.models && typeof entry.models === "object"
        ? Object.keys(entry.models as Record<string, unknown>)
        : [];
    const catalogApi = typeof entry.api === "string" && entry.api.length > 0 ? entry.api : null;
    const fallback = KNOWN_BASE_URLS[id] ?? null;
    providers.push({
      id,
      name: typeof entry.name === "string" && entry.name.length > 0 ? entry.name : id,
      api: catalogApi ?? fallback,
      apiFromFallback: catalogApi === null && fallback !== null,
      doc: typeof entry.doc === "string" && entry.doc.length > 0 ? entry.doc : null,
      models,
    });
  }
  providers.sort((a, b) => a.name.localeCompare(b.name));
  return providers;
}

/** USD per million tokens, matching the server's `ModelPrice` shape. */
export interface ModelPrice {
  input: number;
  output: number;
  cache_read: number;
  cache_write: number;
}

/**
 * Model prices from the models.dev catalog, keyed by model id. The catalog
 * records `cost: { input, output, cache_read?, cache_write? }` in USD per
 * million tokens on each model. When several providers list the same model
 * id, the first non-zero price wins — providers reselling a model rarely
 * disagree, and the analytics page allows manual correction anyway.
 */
export async function loadModelsDevPrices(): Promise<Record<string, ModelPrice>> {
  const resp = await fetch("https://models.dev/api.json");
  if (!resp.ok) {
    throw new Error(`models.dev catalog fetch failed: ${resp.status}`);
  }
  const data = (await resp.json()) as Record<string, unknown>;
  const prices: Record<string, ModelPrice> = {};
  for (const raw of Object.values(data)) {
    if (!raw || typeof raw !== "object") continue;
    const models = (raw as { models?: unknown }).models;
    if (!models || typeof models !== "object") continue;
    for (const [modelId, modelRaw] of Object.entries(models as Record<string, unknown>)) {
      if (prices[modelId]) continue;
      const cost = (modelRaw as { cost?: unknown })?.cost;
      if (!cost || typeof cost !== "object") continue;
      const c = cost as Record<string, unknown>;
      const num = (v: unknown) => (typeof v === "number" && v >= 0 ? v : 0);
      const price: ModelPrice = {
        input: num(c.input),
        output: num(c.output),
        cache_read: num(c.cache_read),
        cache_write: num(c.cache_write),
      };
      if (price.input > 0 || price.output > 0) {
        prices[modelId] = price;
      }
    }
  }
  return prices;
}
