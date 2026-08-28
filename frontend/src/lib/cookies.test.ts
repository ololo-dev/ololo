import { describe, expect, it, vi } from "vitest";
import { extractSetCookies, parseSetCookie, forwardSetCookies } from "./cookies";

function headersWithGetSetCookie(cookies: string[]): Headers {
  const h = new Headers();
  (h as unknown as { getSetCookie: () => string[] }).getSetCookie = () => cookies;
  return h;
}

// Minimal Headers stand-in WITHOUT a native getSetCookie, to exercise the
// fallback split path in extractSetCookies.
function plainHeaders(setCookie?: string): {
  getSetCookie?: never;
  get(name: string): string | null;
} {
  return {
    get(name: string) {
      return name.toLowerCase() === "set-cookie" ? (setCookie ?? null) : null;
    },
  } as { getSetCookie?: never; get(name: string): string | null };
}

describe("extractSetCookies", () => {
  it("prefers getSetCookie when available", () => {
    const h = headersWithGetSetCookie(["a=1", "b=2"]);
    expect(extractSetCookies(h)).toEqual(["a=1", "b=2"]);
  });

  it("falls back to splitting a combined header, keeping Expires commas", () => {
    const h = plainHeaders("a=1; Expires=Wed, 09 Jun 2027 10:18:14 GMT, b=2; Path=/");
    expect(extractSetCookies(h as unknown as Headers)).toEqual([
      "a=1; Expires=Wed, 09 Jun 2027 10:18:14 GMT",
      " b=2; Path=/",
    ]);
  });

  it("returns empty when no set-cookie header and no getSetCookie", () => {
    expect(extractSetCookies(plainHeaders() as unknown as Headers)).toEqual([]);
  });
});

describe("parseSetCookie", () => {
  it("returns null for empty input", () => {
    expect(parseSetCookie("")).toBeNull();
  });

  it("returns null when there is no '='", () => {
    expect(parseSetCookie("garbage")).toBeNull();
  });

  it("decodes the value and parses all options", () => {
    const parsed = parseSetCookie(
      "sess=hello%20world; Path=/api; Domain=example.com; Expires=Wed, 09 Jun 2027 10:18:14 GMT; Max-Age=3600; HttpOnly; Secure; SameSite=Lax",
    );
    expect(parsed).toEqual({
      name: "sess",
      value: "hello world",
      options: {
        path: "/api",
        domain: "example.com",
        expires: new Date("Wed, 09 Jun 2027 10:18:14 GMT"),
        maxAge: 3600,
        httpOnly: true,
        secure: true,
        sameSite: "lax",
      },
    });
  });

  it("accepts SameSite strict and none case-insensitively", () => {
    expect(parseSetCookie("a=1; SameSite=STRICT")!.options.sameSite).toBe("strict");
    expect(parseSetCookie("a=1; samesite=None")!.options.sameSite).toBe("none");
  });

  it("skips empty attribute values for expires/max-age/samesite", () => {
    expect(parseSetCookie("a=1; Expires=; Max-Age=; SameSite=")!.options).toEqual({});
  });
});

describe("forwardSetCookies", () => {
  function makeCookies() {
    const set = vi.fn();
    const del = vi.fn();
    const cookies = { set, delete: del } as unknown as import("@sveltejs/kit").Cookies;
    return { cookies, set, del };
  }

  function upstream(cookies: string[]): Response {
    return { headers: headersWithGetSetCookie(cookies) } as unknown as Response;
  }

  it("sets cookies with default path and forwarded options", () => {
    const { cookies, set, del } = makeCookies();
    forwardSetCookies(
      upstream(["sess=v; Domain=d.com; Max-Age=60; HttpOnly; Secure; SameSite=none"]),
      cookies,
    );
    expect(del).not.toHaveBeenCalled();
    expect(set).toHaveBeenCalledWith("sess", "v", {
      path: "/",
      domain: "d.com",
      maxAge: 60,
      httpOnly: true,
      secure: true,
      sameSite: "none",
    });
  });

  it("deletes cookies with empty value and zero max-age", () => {
    const { cookies, set, del } = makeCookies();
    forwardSetCookies(upstream(["sess=; Max-Age=0"]), cookies);
    expect(set).not.toHaveBeenCalled();
    expect(del).toHaveBeenCalledWith("sess", { path: "/" });
  });

  it("skips unparseable cookies but forwards valid ones", () => {
    const { cookies, set, del } = makeCookies();
    forwardSetCookies(upstream(["no-equals", "ok=1"]), cookies);
    expect(set).toHaveBeenCalledTimes(1);
    expect(del).not.toHaveBeenCalled();
  });
});
