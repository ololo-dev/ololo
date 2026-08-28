import { afterEach, describe, expect, it } from "vitest";
import { render, fireEvent, cleanup } from "@testing-library/svelte";
import TraceTimeline from "./TraceTimeline.svelte";
import type { JudgeLogEvent } from "$lib/types/arena";

/** Minimal event; individual tests override just what they exercise. */
function ev(over: Partial<JudgeLogEvent> = {}): JudgeLogEvent {
  return { at_ms: 0, kind: "llm", duration_ms: 100, ...over } as JudgeLogEvent;
}

/** `events` is a reserved testing-library option, so props must be explicit. */
function renderTimeline(events: JudgeLogEvent[]) {
  return render(TraceTimeline, { props: { events } });
}

/** The row header is the clickable disclosure for observation `i`. */
async function expand(getByTestId: (id: string) => HTMLElement, i = 0) {
  await fireEvent.click(getByTestId(`trace-event-${i}`).querySelector("button")!);
}

describe("TraceTimeline", () => {
  afterEach(cleanup);

  it("orders observations by start time, not array order", () => {
    // The enveloping LLM event is recorded when the agent loop finishes but
    // stamped with its start, so it arrives last while belonging first.
    const { getByTestId } = renderTimeline([
      ev({ at_ms: 5_000, kind: "tool", name: "read_file" }),
      ev({ at_ms: 1_000, model: "claude-opus-4-8" }),
    ]);
    expect(getByTestId("trace-event-0").textContent).toContain("claude-opus-4-8");
    expect(getByTestId("trace-event-1").textContent).toContain("read_file");
  });

  it("names the stages of a run that never called a model", async () => {
    // A judge run is not only model turns. The evidence it read, the decision
    // its own program made and each probe a sandbox re-ran are stages too —
    // titled "LLM turn" they were indistinguishable from one another.
    const { getByTestId, getByText } = renderTimeline([
      ev({ at_ms: 0, kind: "evidence", name: "snapshot", output: '{"task":{}}' }),
      ev({ at_ms: 100, kind: "decide", name: "program", output: "Skip" }),
      ev({ at_ms: 200, kind: "probe", name: "probe #2", output: "138" }),
    ]);
    expect(getByTestId("trace-event-0").textContent).toContain("snapshot");
    expect(getByTestId("trace-event-1").textContent).toContain("program");
    expect(getByTestId("trace-event-2").textContent).toContain("probe #2");
    expect(getByTestId("trace-event-0").textContent).not.toContain("LLM turn");

    // And their payload is not a completion, so it must not say it is.
    await expand(getByTestId, 2);
    expect(getByText("Output")).toBeTruthy();
  });

  it("shows offsets relative to the first observation", () => {
    const { getByTestId } = renderTimeline([
      ev({ at_ms: 10_000 }),
      ev({ at_ms: 12_500, kind: "tool", name: "grep" }),
    ]);
    expect(getByTestId("trace-event-0").textContent).toContain("+0.0s");
    expect(getByTestId("trace-event-1").textContent).toContain("+2.5s");
  });

  it("keeps payloads collapsed until the row is clicked", async () => {
    const { getByTestId, queryByText } = renderTimeline([
      ev({ input: "the-prompt", output: "the-completion" }),
    ]);
    expect(queryByText("the-prompt")).toBeNull();

    await expand(getByTestId);
    expect(queryByText("the-prompt")).toBeTruthy();
    expect(queryByText("the-completion")).toBeTruthy();
  });

  it("renders a readable transcript with tool calls, and toggles to raw JSON", async () => {
    const { getByTestId, getByText, queryByText } = renderTimeline([
      ev({
        messages: [
          { role: "user", content: "review this diff" },
          {
            role: "assistant",
            content: [
              { text: "let me look" },
              { function: { name: "read_file", arguments: '{"path":"a.rs"}' } },
            ],
          },
        ],
      }),
    ]);
    await expand(getByTestId);

    expect(getByText("Transcript (2 turns)")).toBeTruthy();
    expect(queryByText("review this diff")).toBeTruthy();
    expect(queryByText("read_file()")).toBeTruthy();
    // Args are pretty-printed from the JSON string rig hands us.
    expect(queryByText(/"path": "a\.rs"/)).toBeTruthy();

    await fireEvent.click(getByText("Raw JSON"));
    expect(queryByText("read_file()")).toBeNull();
    expect(getByText("Readable")).toBeTruthy();
  });

  it("falls back to raw JSON when the transcript is not an array of turns", async () => {
    const { getByTestId, getByText, queryByText } = renderTimeline([
      ev({ messages: { unexpected: "shape" } as never }),
    ]);
    await expand(getByTestId);

    // No turn count, and no Readable/Raw toggle — there is nothing to read.
    expect(getByText("Transcript")).toBeTruthy();
    expect(queryByText("Raw JSON")).toBeNull();
    expect(queryByText(/"unexpected": "shape"/)).toBeTruthy();
  });

  it("surfaces errors on the row and in the body", async () => {
    const { getByTestId, getByText, queryByText } = renderTimeline([
      ev({ error: "429 rate limited" }),
    ]);
    expect(getByTestId("trace-event-0").textContent).toContain("error");

    await expand(getByTestId);
    expect(getByText("Error")).toBeTruthy();
    expect(queryByText("429 rate limited")).toBeTruthy();
  });

  it("reports observations with nothing captured rather than rendering blank", async () => {
    const { getByTestId, queryByText } = renderTimeline([ev()]);
    await expand(getByTestId);
    expect(queryByText("No payload captured for this observation.")).toBeTruthy();
  });
});
