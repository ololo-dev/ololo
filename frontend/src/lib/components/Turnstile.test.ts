import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "@testing-library/svelte";
import Turnstile from "./Turnstile.svelte";

vi.mock("$app/environment", () => ({
  browser: true,
}));

describe("Turnstile", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    document.body.innerHTML = "";
  });

  it("renders cf-turnstile div when in browser", () => {
    render(Turnstile, { sitekey: "test-sitekey", ontoken: vi.fn() });
    const div = document.querySelector(".turnstile-widget");
    expect(div).toBeTruthy();
  });

  it("accepts sitekey and ontoken props without error", () => {
    const ontoken = vi.fn();
    expect(() => render(Turnstile, { sitekey: "1x00000000000000000000AA", ontoken })).not.toThrow();
  });
});
