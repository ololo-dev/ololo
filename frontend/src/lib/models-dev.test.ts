import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";

/** Shape of the real api.json, trimmed to what the parser reads. */
const CATALOG = {
  // Has its own endpoint — the catalog wins.
  "fireworks-ai": {
    name: "Fireworks AI",
    api: "https://api.fireworks.ai/inference/v1/",
    doc: "https://docs.fireworks.ai",
    models: { "model-a": {}, "model-b": {} },
  },
  // No endpoint in the catalog, but we know it.
  groq: {
    name: "Groq",
    doc: "https://console.groq.com/docs/models",
    models: { "llama-3.3-70b-versatile": {} },
  },
  cerebras: {
    name: "Cerebras",
    doc: "https://inference-docs.cerebras.ai/models/overview",
    models: {},
  },
  // Endpoint is per-resource, so nobody can suggest one.
  azure: { name: "Azure", doc: "https://azure.example", models: {} },
  // Degenerate entries the parser must survive.
  blank: { name: "", api: "", models: {} },
};

beforeEach(() => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => new Response(JSON.stringify(CATALOG), { status: 200 })),
  );
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.resetModules();
});

/** The loader memoises per module instance, so reimport for a clean cache. */
async function load() {
  vi.resetModules();
  const mod = await import("./models-dev");
  return mod.loadModelsDevCatalog();
}

describe("models.dev catalog", () => {
  it("uses the catalog's own api field when present", async () => {
    const byId = Object.fromEntries((await load()).map((p) => [p.id, p]));
    expect(byId["fireworks-ai"].api).toBe("https://api.fireworks.ai/inference/v1/");
    expect(byId["fireworks-ai"].apiFromFallback).toBe(false);
  });

  it("fills in known endpoints the catalog omits", async () => {
    // models.dev leaves `api` blank for providers whose SDK hardcodes it,
    // which is most of the popular ones. Without the fallback, picking Groq
    // from the catalog left Base URL empty and the save was rejected.
    const byId = Object.fromEntries((await load()).map((p) => [p.id, p]));
    expect(byId.groq.api).toBe("https://api.groq.com/openai/v1");
    expect(byId.groq.apiFromFallback).toBe(true);
    expect(byId.cerebras.api).toBe("https://api.cerebras.ai/v1");
    expect(byId.cerebras.apiFromFallback).toBe(true);
  });

  it("leaves per-resource providers without a guessed endpoint", async () => {
    // Azure's URL depends on the customer's own resource; inventing one
    // would be worse than an empty field the admin must fill.
    const byId = Object.fromEntries((await load()).map((p) => [p.id, p]));
    expect(byId.azure.api).toBeNull();
    expect(byId.azure.apiFromFallback).toBe(false);
    expect(byId.azure.doc).toBe("https://azure.example");
  });

  it("keeps names, docs and model ids, and tolerates blank fields", async () => {
    const byId = Object.fromEntries((await load()).map((p) => [p.id, p]));
    expect(byId["fireworks-ai"].models).toEqual(["model-a", "model-b"]);
    expect(byId.groq.doc).toBe("https://console.groq.com/docs/models");
    // Empty strings are not usable values: name falls back to the id, api to null.
    expect(byId.blank.name).toBe("blank");
    expect(byId.blank.api).toBeNull();
  });

  it("fetches once and reuses the parsed catalog", async () => {
    // The file is ~3 MB, so a second call must not refetch it.
    vi.resetModules();
    const mod = await import("./models-dev");
    await mod.loadModelsDevCatalog();
    await mod.loadModelsDevCatalog();
    expect(vi.mocked(fetch)).toHaveBeenCalledTimes(1);
  });

  it("clears the cache after a failure so a retry can succeed", async () => {
    vi.resetModules();
    vi.stubGlobal(
      "fetch",
      vi
        .fn()
        .mockResolvedValueOnce(new Response("nope", { status: 500 }))
        .mockResolvedValueOnce(new Response(JSON.stringify(CATALOG), { status: 200 })),
    );
    const mod = await import("./models-dev");
    await expect(mod.loadModelsDevCatalog()).rejects.toThrow();
    const second = await mod.loadModelsDevCatalog();
    expect(second.length).toBeGreaterThan(0);
  });
});

describe("providerLogoId", () => {
  it("prefers the catalog id when the provider has one", async () => {
    const { providerLogoId } = await import("./models-dev");
    expect(providerLogoId({ catalog_id: "groq", kind: "openai_compatible" })).toBe("groq");
    // An Ollama entry created from the catalog keeps its own id.
    expect(providerLogoId({ catalog_id: "ollama", kind: "ollama" })).toBe("ollama");
  });

  it("falls back to the kind for a catalog-less Ollama daemon", async () => {
    const { providerLogoId } = await import("./models-dev");
    expect(providerLogoId({ catalog_id: null, kind: "ollama" })).toBe("ollama");
  });

  it("has no logo for any other catalog-less provider", async () => {
    const { providerLogoId } = await import("./models-dev");
    expect(providerLogoId({ catalog_id: null, kind: "openai_compatible" })).toBeNull();
    expect(providerLogoId({ catalog_id: null, kind: "openrouter" })).toBeNull();
  });
});
