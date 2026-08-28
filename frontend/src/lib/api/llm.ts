// Admin LLM provider registry and per-operation model assignments.

import { request, type FetchLike } from "./core";

export type LlmProviderKind = "ollama" | "openrouter" | "openai_compatible";

export interface LlmProvider {
  id: string;
  name: string;
  kind: LlmProviderKind;
  base_url: string | null;
  /** The stored API key is never echoed back; only its presence. */
  has_api_key: boolean;
  enabled: boolean;
  /** models.dev catalog provider id this entry was created from, if any. */
  catalog_id: string | null;
  created_at: string;
  updated_at: string;
}

export interface CreateLlmProviderBody {
  name: string;
  kind: LlmProviderKind;
  base_url?: string;
  api_key?: string;
  enabled?: boolean;
  catalog_id?: string;
}

export interface UpdateLlmProviderBody {
  name?: string;
  kind?: LlmProviderKind;
  base_url?: string;
  /** Absent = keep the stored key. */
  api_key?: string;
  clear_api_key?: boolean;
  enabled?: boolean;
  catalog_id?: string;
}

/** Operations that can have a dedicated provider/model assignment. */
export type LlmOperation = "judge" | "memory" | "adaptation" | "project_ai";

/** One provider row + one model id. */
export interface LlmModelAssignment {
  provider_id: string;
  model: string;
}

/** Points at a pool; the pool's members become the candidates. */
export interface LlmPoolAssignment {
  pool_id: string;
}

/**
 * An assignment is either a single model or a whole pool. The two are told
 * apart by their fields — the single-model shape is unchanged from before
 * pools existed.
 */
export type LlmAssignment = LlmModelAssignment | LlmPoolAssignment;

export function isPoolAssignment(a: LlmAssignment | null | undefined): a is LlmPoolAssignment {
  return !!a && "pool_id" in a;
}

export function isModelAssignment(a: LlmAssignment | null | undefined): a is LlmModelAssignment {
  return !!a && "provider_id" in a;
}

export interface LlmAssignments {
  default: LlmAssignment | null;
  operations: Partial<Record<LlmOperation, LlmAssignment>>;
}

export interface UpdateLlmAssignmentsBody {
  /** Absent = unchanged; null clears. */
  default?: LlmAssignment | null;
  /** Per-op: absent = unchanged; null clears (inherit default). */
  operations?: Partial<Record<LlmOperation, LlmAssignment | null>>;
}

export function listLlmProviders(opts: { fetch?: FetchLike } = {}): Promise<LlmProvider[]> {
  return request<LlmProvider[]>("/api/admin/llm/providers", { fetch: opts.fetch });
}

export function createLlmProvider(
  body: CreateLlmProviderBody,
  opts: { fetch?: FetchLike } = {},
): Promise<LlmProvider> {
  return request<LlmProvider>("/api/admin/llm/providers", {
    method: "POST",
    body,
    fetch: opts.fetch,
  });
}

export function updateLlmProvider(
  id: string,
  body: UpdateLlmProviderBody,
  opts: { fetch?: FetchLike } = {},
): Promise<LlmProvider> {
  return request<LlmProvider>(`/api/admin/llm/providers/${encodeURIComponent(id)}`, {
    method: "PUT",
    body,
    fetch: opts.fetch,
  });
}

export function deleteLlmProvider(id: string, opts: { fetch?: FetchLike } = {}): Promise<void> {
  return request<void>(`/api/admin/llm/providers/${encodeURIComponent(id)}`, {
    method: "DELETE",
    fetch: opts.fetch,
  });
}

export interface LlmProviderTestResult {
  ok: boolean;
  model: string;
  latency_ms: number;
  /** First 300 chars of the model's reply, on success. */
  output?: string | null;
  /** The provider's own error text, unredacted, on failure. */
  error?: string | null;
}

/** One real completion through the stored credentials — the paid endpoint,
 *  not the models listing, so dead keys and empty balances surface here. */
export function testLlmProvider(
  id: string,
  body: { model: string },
  opts: { fetch?: FetchLike } = {},
): Promise<LlmProviderTestResult> {
  return request<LlmProviderTestResult>(`/api/admin/llm/providers/${encodeURIComponent(id)}/test`, {
    method: "POST",
    body,
    fetch: opts.fetch,
  });
}

/** Live model ids reported by the provider; `[]` when unreachable. */
export function listLlmProviderModels(
  id: string,
  opts: { fetch?: FetchLike } = {},
): Promise<string[]> {
  return request<string[]>(`/api/admin/llm/providers/${encodeURIComponent(id)}/models`, {
    fetch: opts.fetch,
  });
}

export function getLlmAssignments(opts: { fetch?: FetchLike } = {}): Promise<LlmAssignments> {
  return request<LlmAssignments>("/api/admin/llm/assignments", { fetch: opts.fetch });
}

export function updateLlmAssignments(
  body: UpdateLlmAssignmentsBody,
  opts: { fetch?: FetchLike } = {},
): Promise<LlmAssignments> {
  return request<LlmAssignments>("/api/admin/llm/assignments", {
    method: "PUT",
    body,
    fetch: opts.fetch,
  });
}

// ---------- Pools ----------

export interface LlmPoolMember {
  id: string;
  provider_id: string;
  model: string;
  /** Lower is tried first; members sharing a value form one load-split tier. */
  priority: number;
  enabled: boolean;
}

export interface LlmPool {
  id: string;
  name: string;
  description: string;
  /** Already ordered by ascending priority — the resolution order. */
  members: LlmPoolMember[];
  created_at: string;
  updated_at: string;
}

/** A member as sent on write — the server assigns member ids. */
export interface LlmPoolMemberInput {
  provider_id: string;
  model: string;
  priority?: number;
  enabled?: boolean;
}

export interface CreateLlmPoolBody {
  name: string;
  description?: string;
  members?: LlmPoolMemberInput[];
}

export interface UpdateLlmPoolBody {
  name?: string;
  description?: string;
  /** Absent = members unchanged; present = replaces the list wholesale. */
  members?: LlmPoolMemberInput[];
}

export function listLlmPools(opts: { fetch?: FetchLike } = {}): Promise<LlmPool[]> {
  return request<LlmPool[]>("/api/admin/llm/pools", { fetch: opts.fetch });
}

export function createLlmPool(
  body: CreateLlmPoolBody,
  opts: { fetch?: FetchLike } = {},
): Promise<LlmPool> {
  return request<LlmPool>("/api/admin/llm/pools", {
    method: "POST",
    body,
    fetch: opts.fetch,
  });
}

export function updateLlmPool(
  id: string,
  body: UpdateLlmPoolBody,
  opts: { fetch?: FetchLike } = {},
): Promise<LlmPool> {
  return request<LlmPool>(`/api/admin/llm/pools/${encodeURIComponent(id)}`, {
    method: "PUT",
    body,
    fetch: opts.fetch,
  });
}

export function deleteLlmPool(id: string, opts: { fetch?: FetchLike } = {}): Promise<void> {
  return request<void>(`/api/admin/llm/pools/${encodeURIComponent(id)}`, {
    method: "DELETE",
    fetch: opts.fetch,
  });
}

// ---------- Telemetry ----------

/** Operations recorded in LLM telemetry: the assignable ops plus task AI. */
export type LlmTelemetryOperation = LlmOperation | "task_ai";

export type LlmTelemetryStatus = "ok" | "failed";

export interface LlmTelemetryRow {
  id: string;
  operation: LlmTelemetryOperation;
  /**
   * Provider registry id (e.g. "ollama", "openrouter", "custom"). Every
   * OpenAI-compatible endpoint reports "custom", so this does not identify
   * which one ran — use `provider_name` for display.
   */
  provider: string;
  /** Name of the configured provider row, when the call went through one. */
  provider_name: string | null;
  model: string;
  status: LlmTelemetryStatus;
  error: string | null;
  tokens_input: number;
  tokens_output: number;
  tokens_cache_read: number;
  tokens_cache_write: number;
  duration_ms: number;
  session_id: string | null;
  player_id: string | null;
  task_id: string | null;
  /** Join code for `session_id`; null once the session is gone. */
  session_code: string | null;
  /** The player's display name. */
  player_name: string | null;
  /** Account username, when the player was signed in — linkable as `/u/{it}`. */
  player_username: string | null;
  task_title: string | null;
  judge_slug: string | null;
  /** JSON string with operation-specific details (sizes, excerpts, events). */
  detail_json: string | null;
  /** JSON-encoded array of per-turn trace events (`JudgeLogEvent[]`).
   * Populated only by the single-row read — list rows always have null. */
  events_json: string | null;
  created_at: string;
  /** USD from the model-price map; null when the model is unpriced. */
  cost?: number | null;
}

export interface LlmTelemetryPage {
  items: LlmTelemetryRow[];
  total: number;
}

export interface LlmTelemetryQuery {
  operation?: LlmTelemetryOperation;
  status?: LlmTelemetryStatus;
  /** Page size, default 50, max 200. */
  limit?: number;
  offset?: number;
}

/** Admin-only: paged LLM call telemetry, newest first. */
export function getLlmTelemetry(
  params: LlmTelemetryQuery = {},
  opts: { fetch?: FetchLike } = {},
): Promise<LlmTelemetryPage> {
  const qs = new URLSearchParams();
  if (params.operation) qs.set("operation", params.operation);
  if (params.status) qs.set("status", params.status);
  if (params.limit != null) qs.set("limit", String(params.limit));
  if (params.offset != null) qs.set("offset", String(params.offset));
  const query = qs.toString();
  return request<LlmTelemetryPage>(`/api/admin/llm/telemetry${query ? `?${query}` : ""}`, {
    fetch: opts.fetch,
  });
}

/** Admin-only: one telemetry record by id (404 when missing). */
export function getLlmTelemetryRecord(
  id: string,
  opts: { fetch?: FetchLike } = {},
): Promise<LlmTelemetryRow> {
  return request<LlmTelemetryRow>(`/api/admin/llm/telemetry/${encodeURIComponent(id)}`, {
    fetch: opts.fetch,
  });
}
