// Admin cost analytics: platform LLM spending aggregated from telemetry.

import { request, type FetchLike } from "./core";

/** USD per million tokens, by token kind. */
export interface ModelPrice {
  input: number;
  output: number;
  cache_read: number;
  cache_write: number;
}

/** One aggregated row of spending. */
export interface CostBucket {
  key: string;
  label?: string | null;
  /** Secondary line under the label (a session's start date). */
  sublabel?: string | null;
  requests: number;
  failed_requests: number;
  tokens_input: number;
  tokens_output: number;
  tokens_cache_read: number;
  tokens_cache_write: number;
  /** USD over priced models; null when nothing in the bucket was priced. */
  cost: number | null;
  unpriced_requests: number;
}

/** One day×hour cell of the usage heatmap (UTC). */
export interface HeatCell {
  day: string;
  hour: number;
  requests: number;
  tokens: number;
  cost: number | null;
}

export interface CostsSummary {
  days: number;
  totals: CostBucket;
  by_model: CostBucket[];
  by_judge: CostBucket[];
  by_operation: CostBucket[];
  by_session: CostBucket[];
  /** Spending per player account across every session in the window. */
  by_player?: CostBucket[];
  /** Day×hour usage grid (UTC); empty cells omitted. */
  heatmap?: HeatCell[];
  prices: Record<string, ModelPrice>;
  unpriced_models: string[];
}

export interface SessionCosts {
  session_id: string;
  join_code: string;
  totals: CostBucket;
  by_player: CostBucket[];
  by_judge: CostBucket[];
  by_operation: CostBucket[];
}

export function getCostsSummary(days: number, opts: { fetch?: FetchLike } = {}) {
  return request<CostsSummary>(`/api/admin/analytics/costs?days=${days}`, { fetch: opts.fetch });
}

export function getSessionCosts(sessionId: string, opts: { fetch?: FetchLike } = {}) {
  return request<SessionCosts>(`/api/admin/sessions/${encodeURIComponent(sessionId)}/costs`, {
    fetch: opts.fetch,
  });
}
