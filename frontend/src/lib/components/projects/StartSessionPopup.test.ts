import { afterEach, describe, expect, it } from "vitest";
import { render, cleanup } from "@testing-library/svelte";
import StartSessionPopup from "./StartSessionPopup.svelte";

describe("StartSessionPopup", () => {
  afterEach(() => cleanup());

  it("renders the start command when open with a slug", () => {
    const { getByText, getByRole } = render(StartSessionPopup, {
      slug: "my-project",
      open: true,
    });
    expect(getByRole("dialog", { name: "Start session" })).toBeTruthy();
    expect(getByText("ololo start my-project")).toBeTruthy();
  });

  it("does not render when closed", () => {
    const { queryByRole } = render(StartSessionPopup, { slug: "my-project", open: false });
    expect(queryByRole("dialog")).toBeNull();
  });

  it("does not render without a slug even when open", () => {
    const { queryByRole } = render(StartSessionPopup, { slug: "", open: true });
    expect(queryByRole("dialog")).toBeNull();
  });
});
