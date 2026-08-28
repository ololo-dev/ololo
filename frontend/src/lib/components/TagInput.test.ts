import { afterEach, describe, expect, it, vi } from "vitest";
import { render, fireEvent, cleanup } from "@testing-library/svelte";
import TagInput from "./TagInput.svelte";

describe("TagInput", () => {
  afterEach(() => cleanup());

  it("adds a tag on Enter and reports via onchange", async () => {
    const onchange = vi.fn();
    const { getByPlaceholderText } = render(TagInput, { tags: [], onchange });
    const input = getByPlaceholderText("Add tags (Enter or comma)") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "alpha" } });
    await fireEvent.keyDown(input, { key: "Enter" });
    expect(onchange).toHaveBeenCalledWith(["alpha"]);
  });

  it("adds a tag when a comma is typed", async () => {
    const onchange = vi.fn();
    const { getByPlaceholderText } = render(TagInput, { tags: [], onchange });
    const input = getByPlaceholderText("Add tags (Enter or comma)") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "beta," } });
    expect(onchange).toHaveBeenCalledWith(["beta"]);
  });

  it("trims a trailing comma from the typed value", async () => {
    const onchange = vi.fn();
    const { getByPlaceholderText } = render(TagInput, { tags: [], onchange });
    const input = getByPlaceholderText("Add tags (Enter or comma)") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "gamma," } });
    await fireEvent.keyDown(input, { key: "," });
    expect(onchange).toHaveBeenCalledWith(["gamma"]);
  });

  it("ignores empty and duplicate tags", async () => {
    const onchange = vi.fn();
    const { getByPlaceholderText } = render(TagInput, { tags: ["a"], onchange });
    const input = getByPlaceholderText("Add tag…") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "   " } });
    await fireEvent.keyDown(input, { key: "Enter" });
    await fireEvent.input(input, { target: { value: "a" } });
    await fireEvent.keyDown(input, { key: "Enter" });
    expect(onchange).not.toHaveBeenCalled();
  });

  it("ignores tags longer than 50 characters", async () => {
    const onchange = vi.fn();
    const { getByPlaceholderText } = render(TagInput, { tags: [], onchange });
    const input = getByPlaceholderText("Add tags (Enter or comma)") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "x".repeat(51) } });
    await fireEvent.keyDown(input, { key: "Enter" });
    expect(onchange).not.toHaveBeenCalled();
  });

  it("removes a tag via its remove button", async () => {
    const onchange = vi.fn();
    const { getByLabelText } = render(TagInput, { tags: ["a", "b"], onchange });
    await fireEvent.click(getByLabelText("Remove tag a"));
    expect(onchange).toHaveBeenCalledWith(["b"]);
  });

  it("hides the input once ten tags exist", () => {
    const { queryByPlaceholderText } = render(TagInput, {
      tags: Array.from({ length: 10 }, (_, i) => `t${i}`),
    });
    expect(queryByPlaceholderText("Add tag…")).toBeNull();
  });

  it("adds a tenth tag when nine already exist", async () => {
    const onchange = vi.fn();
    const { getByPlaceholderText } = render(TagInput, {
      tags: Array.from({ length: 9 }, (_, i) => `t${i}`),
      onchange,
    });
    const input = getByPlaceholderText("Add tag…") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "tenth" } });
    await fireEvent.keyDown(input, { key: "Enter" });
    expect(onchange).toHaveBeenCalledWith([
      "t0",
      "t1",
      "t2",
      "t3",
      "t4",
      "t5",
      "t6",
      "t7",
      "t8",
      "tenth",
    ]);
  });
});
