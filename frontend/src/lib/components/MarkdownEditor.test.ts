import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import MarkdownEditor from "./MarkdownEditor.svelte";

describe("MarkdownEditor", () => {
  it("wraps the selection when a toolbar tool runs", async () => {
    const oninput = vi.fn();
    render(MarkdownEditor, { value: "make me strong", oninput, name: "body_md" });
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    textarea.setSelectionRange(8, 14);
    await fireEvent.click(screen.getByTitle("Bold"));
    expect(oninput).toHaveBeenCalledWith("make me **strong**");
  });

  it("prefixes every selected line for list tools", async () => {
    const oninput = vi.fn();
    render(MarkdownEditor, { value: "one\ntwo", oninput });
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    textarea.setSelectionRange(0, 7);
    await fireEvent.click(screen.getByTitle("Numbered list"));
    expect(oninput).toHaveBeenCalledWith("1. one\n2. two");
  });

  it("renders the markdown in preview and keeps the form control around", async () => {
    render(MarkdownEditor, { value: "## Hello", oninput: () => {}, name: "body_md" });
    await fireEvent.click(screen.getByTestId("md-preview"));
    const pane = screen.getByTestId("md-preview-pane");
    expect(pane.querySelector("h2")?.textContent).toBe("Hello");
    // Hidden, not removed: the textarea still submits with the form.
    const textarea = document.querySelector('textarea[name="body_md"]');
    expect(textarea).not.toBeNull();
  });
});
