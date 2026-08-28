import { describe, it, expect } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import DiffView from "./DiffView.svelte";
import type { PlayerHistoryFileDiff } from "$lib/types/arena";

function mkFile(
  path: string,
  status: "added" | "modified" | "deleted",
  patch: string,
): PlayerHistoryFileDiff {
  return { path, status, patch };
}

describe("DiffView", () => {
  it("renders file header (collapsed), expand to see lines", async () => {
    const files = [mkFile("a.txt", "added", "+hello\n+world")];
    const { container, getByText } = render(DiffView, { props: { files } });
    expect(getByText("a.txt")).toBeTruthy();
    expect(getByText("added")).toBeTruthy();
    // Patch hidden when collapsed
    expect(container.textContent).not.toContain("+hello");
    // Click to expand
    await fireEvent.click(getByText("a.txt"));
    expect(container.textContent).toContain("+hello");
    expect(container.textContent).toContain("+world");
  });

  it("renders multiple file headers", () => {
    const files = [
      mkFile("a.txt", "added", "+a"),
      mkFile("b.txt", "modified", "@@ -1 +1 @@\n-b\n+B"),
    ];
    const { getByText } = render(DiffView, { props: { files } });
    expect(getByText("a.txt")).toBeTruthy();
    expect(getByText("b.txt")).toBeTruthy();
  });

  it("empty files array → empty container", () => {
    const { container } = render(DiffView, { props: { files: [] } });
    expect(container.querySelector('[data-testid="diff-view"]')).toBeTruthy();
    expect(container.textContent?.trim()).toBe("");
  });

  it("colors added lines green and removed lines red after expand", async () => {
    const files = [mkFile("a.txt", "modified", "@@ -1 +1 @@\n-old\n+new")];
    const { container, getByText } = render(DiffView, { props: { files } });
    await fireEvent.click(getByText("a.txt"));
    const spans = Array.from(container.querySelectorAll("span"));
    const addLine = spans.find((s) => s.textContent === "+new");
    const delLine = spans.find((s) => s.textContent === "-old");
    expect(addLine).toBeTruthy();
    expect(delLine).toBeTruthy();
    expect(addLine!.className).toContain("green");
    expect(delLine!.className).toContain("red");
  });

  it('has data-testid="diff-view" on root', () => {
    const { container } = render(DiffView, { props: { files: [] } });
    expect(container.querySelector('[data-testid="diff-view"]')).toBeTruthy();
  });
});
