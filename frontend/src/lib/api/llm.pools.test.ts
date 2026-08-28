import { describe, expect, it, vi, beforeEach } from "vitest";
import {
  createLlmPool,
  deleteLlmPool,
  isModelAssignment,
  isPoolAssignment,
  listLlmPools,
  updateLlmPool,
  type LlmAssignment,
} from "./llm";
import { request } from "./core";

vi.mock("./core", () => ({ request: vi.fn() }));

const mockRequest = vi.mocked(request);

beforeEach(() => {
  mockRequest.mockReset();
  mockRequest.mockResolvedValue(undefined as never);
});

describe("pool endpoints", () => {
  it("lists pools", async () => {
    await listLlmPools();
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/llm/pools", { fetch: undefined });
  });

  it("creates a pool with its members", async () => {
    const body = {
      name: "Fast",
      description: "cheap first",
      members: [{ provider_id: "p1", model: "a", priority: 0, enabled: true }],
    };
    await createLlmPool(body);
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/llm/pools", {
      method: "POST",
      body,
      fetch: undefined,
    });
  });

  it("updates a pool, replacing the member list", async () => {
    const body = { members: [{ provider_id: "p2", model: "b" }] };
    await updateLlmPool("pool-1", body);
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/llm/pools/pool-1", {
      method: "PUT",
      body,
      fetch: undefined,
    });
  });

  it("escapes the id in the path", async () => {
    await deleteLlmPool("a/b");
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/llm/pools/a%2Fb", {
      method: "DELETE",
      fetch: undefined,
    });
  });
});

describe("assignment discrimination", () => {
  const pool: LlmAssignment = { pool_id: "pool-1" };
  const model: LlmAssignment = { provider_id: "p1", model: "m" };

  it("tells the two shapes apart", () => {
    expect(isPoolAssignment(pool)).toBe(true);
    expect(isPoolAssignment(model)).toBe(false);
    expect(isModelAssignment(model)).toBe(true);
    expect(isModelAssignment(pool)).toBe(false);
  });

  it("treats absent assignments as neither", () => {
    // Rows that inherit carry null — the guards must not claim those are
    // pools, or an inheriting row would render as a pool selection.
    expect(isPoolAssignment(null)).toBe(false);
    expect(isModelAssignment(null)).toBe(false);
    expect(isPoolAssignment(undefined)).toBe(false);
    expect(isModelAssignment(undefined)).toBe(false);
  });
});
