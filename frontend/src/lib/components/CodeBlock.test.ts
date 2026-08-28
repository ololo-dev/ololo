import { afterEach, describe, expect, it, vi } from "vitest";
import { render, fireEvent, cleanup } from "@testing-library/svelte";
import CodeBlock from "./CodeBlock.svelte";

const env = vi.hoisted(() => ({ browser: true }));

vi.mock("$app/environment", () => ({
  get browser() {
    return env.browser;
  },
}));

describe("CodeBlock", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("renders the provided code", () => {
    const { getByText } = render(CodeBlock, { code: "ololo start abc" });
    expect(getByText("ololo start abc")).toBeTruthy();
  });

  it("copies the code to the clipboard and shows the copied state", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    const { getByLabelText } = render(CodeBlock, { code: "hello" });
    const btn = getByLabelText("Copy to clipboard");
    await fireEvent.click(btn);
    expect(writeText).toHaveBeenCalledWith("hello");
    expect(getByLabelText("Copied!")).toBeTruthy();
  });

  it("is a no-op when not in the browser", async () => {
    env.browser = false;
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    const { getByLabelText } = render(CodeBlock, { code: "hello" });
    await fireEvent.click(getByLabelText("Copy to clipboard"));
    expect(writeText).not.toHaveBeenCalled();
    env.browser = true;
  });
});
