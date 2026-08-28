import { describe, it, expect, vi, beforeEach } from "vitest";

const handleAuthAction = vi.fn(async (..._a: unknown[]) => ({ ok: true }));
vi.mock("$lib/actions/handleAuthAction", () => ({
  handleAuthAction: (...a: unknown[]) => handleAuthAction(...a),
}));

import { actions } from "./+page.server";

function event(fields: Record<string, string>) {
  const data = new FormData();
  for (const [k, v] of Object.entries(fields)) data.append(k, v);
  return {
    request: { formData: async () => data },
    fetch: vi.fn(),
    cookies: {},
  } as never;
}

const bodyOf = () => (handleAuthAction.mock.calls[0][0] as { body: Record<string, unknown> }).body;

beforeEach(() => handleAuthAction.mockClear());

describe("register action", () => {
  const base = { email: "a@b.com", password: "password-12345", display_name: "A" };

  it("Forwards only the fields the register endpoint accepts", async () => {
    // The backend RegisterBody uses deny_unknown_fields — a stray form field
    // forwarded verbatim would turn every registration into a 422.
    await actions.default(event({ ...base, subscribe_newsletter: "1" }));
    expect(Object.keys(bodyOf()).sort()).toEqual(["display_name", "email", "password"]);
  });

  it("Refuses a registration missing its required fields", async () => {
    const result = await actions.default(event({ email: "a@b.com" }) as never);
    expect((result as { status: number }).status).toBe(422);
    expect(handleAuthAction).not.toHaveBeenCalled();
  });
});
