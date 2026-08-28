import { describe, expect, it } from "vitest";
import { ApiError } from "./errors";

describe("ApiError", () => {
  it("builds a default message from status and code", () => {
    const e = new ApiError(404, "not_found", null);
    expect(e).toBeInstanceOf(Error);
    expect(e.name).toBe("ApiError");
    expect(e.status).toBe(404);
    expect(e.code).toBe("not_found");
    expect(e.body).toBeNull();
    expect(e.message).toBe("api 404 (not_found)");
  });

  it("omits the code segment when code is null", () => {
    const e = new ApiError(500, null, { ok: false });
    expect(e.message).toBe("api 500");
    expect(e.body).toEqual({ ok: false });
  });

  it("uses an explicit message when provided", () => {
    const e = new ApiError(401, "unauthorized", null, "nope");
    expect(e.message).toBe("nope");
  });
});
