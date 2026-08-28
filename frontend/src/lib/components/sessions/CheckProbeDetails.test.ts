import { describe, it, expect, vi } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import CheckProbeDetails from "./CheckProbeDetails.svelte";
import type { PlayerProbeEntry } from "$lib/types/arena";

function mkProbe(over: Partial<PlayerProbeEntry> = {}): PlayerProbeEntry {
  return {
    id: "p1",
    task_id: "t1",
    task_title: "Task One",
    task_ordinal: 1,
    adapted_test_id: "at1",
    test_command: "echo hi",
    attempt: 1,
    rendered_command: "echo hi",
    fixture_values: null,
    expected_answer: "hi",
    state: "resolved",
    outcome: "pass",
    actual: "hi",
    expected: "hi",
    exit_code: 0,
    duration_ms: 12,
    dispatched_at: "2026-01-01T00:00:00Z",
    deadline_at: null,
    resolved_at: "2026-01-01T00:00:05Z",
    result: null,
    point_delta: 5,
    updated_at: null,
    ...over,
  };
}

describe("CheckProbeDetails", () => {
  it("opens on the newest run and shows its full values", () => {
    const attempts = [
      mkProbe({ id: "p1", outcome: "error", actual: "nope", point_delta: 0 }),
      mkProbe({ id: "p2", actual: "latest answer" }),
    ];
    const { container, getByText } = render(CheckProbeDetails, {
      props: { attempts, question: null, points: 5 },
    });
    expect(getByText("run 2 of 2")).toBeTruthy();
    expect(container.textContent).toContain("latest answer");
    expect(container.textContent).toContain("passed");
    expect(container.textContent).toContain("echo hi");
  });

  it("pages to the previous run and back", async () => {
    const attempts = [
      mkProbe({ id: "p1", outcome: "error", actual: "first try", point_delta: -2 }),
      mkProbe({ id: "p2", actual: "second try" }),
    ];
    const { container, getByTestId } = render(CheckProbeDetails, {
      props: { attempts, question: null, points: 3 },
    });
    const prev = getByTestId("check-hover-prev");
    const next = getByTestId("check-hover-next");
    expect((next as HTMLButtonElement).disabled).toBe(true);

    await fireEvent.click(prev);
    expect(getByTestId("check-hover-run-label").textContent).toContain("run 1 of 2");
    expect(container.textContent).toContain("first try");
    expect(container.textContent).toContain("failed");
    expect((prev as HTMLButtonElement).disabled).toBe(true);

    await fireEvent.click(next);
    expect(getByTestId("check-hover-run-label").textContent).toContain("run 2 of 2");
    expect(container.textContent).toContain("second try");
  });

  it("hides the pager for a single run", () => {
    const { queryByTestId } = render(CheckProbeDetails, {
      props: { attempts: [mkProbe()], question: null, points: 5 },
    });
    expect(queryByTestId("check-hover-prev")).toBeNull();
    expect(queryByTestId("check-hover-run-label")).toBeNull();
  });

  it("copies the visible run as markdown", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    const attempts = [
      mkProbe({
        id: "p1",
        rendered_command: "curl -s http://localhost/answer",
        fixture_values: JSON.stringify({ city: "Kyiv" }),
      }),
    ];
    const { getByTestId } = render(CheckProbeDetails, {
      props: { attempts, question: "What is the answer?", points: 5 },
    });
    await fireEvent.click(getByTestId("check-hover-copy"));
    expect(writeText).toHaveBeenCalledOnce();
    const text = writeText.mock.calls[0][0] as string;
    expect(text).toContain("### Check: What is the answer?");
    expect(text).toContain("- Status: passed");
    expect(text).toContain("- Expected: hi");
    expect(text).toContain("- Got: hi");
    expect(text).toContain("city: Kyiv");
    expect(text).toContain("curl -s http://localhost/answer");
    vi.unstubAllGlobals();
  });

  it("shows fixtures and vitals of the selected run", () => {
    const attempts = [
      mkProbe({
        fixture_values: JSON.stringify({ n: 42 }),
        exit_code: 1,
        duration_ms: 1500,
        outcome: "error",
        point_delta: -3,
      }),
    ];
    const { container } = render(CheckProbeDetails, {
      props: { attempts, question: null, points: -3 },
    });
    expect(container.textContent).toContain("n");
    expect(container.textContent).toContain("42");
    expect(container.textContent).toContain("exit 1");
    expect(container.textContent).toContain("1.5 s");
    expect(container.textContent).toContain("-3 pts");
  });
});
