import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/svelte";
import JudgesPage from "./+page.svelte";
import type { Judge, JudgeUsage } from "$lib/api";

vi.mock("$app/navigation", () => ({ invalidateAll: vi.fn(async () => {}) }));
vi.mock("$lib/notifications.svelte", () => ({
  notify: { success: vi.fn(), error: vi.fn() },
}));
vi.mock("$lib/llm-models", () => ({ modelSuggestions: async () => [] }));

const judge = (over: Partial<Judge> & { id: string; slug: string }): Judge =>
  ({
    name: over.slug,
    description: "",
    prompt: "Evaluate.",
    rating_scale: { min: 0, max: 10, step: 0.5 },
    llm_provider_id: null,
    llm_model: null,
    llm_pool_id: null,
    llm_source_order: "pool_first",
    avatar_url: null,
    created_at: "2026-08-01T00:00:00Z",
    updated_at: "2026-08-01T00:00:00Z",
    ...over,
  }) as Judge;

const usage = (over: Partial<JudgeUsage> & { judge_id: string }): JudgeUsage => ({
  attachments: [],
  stats: {
    verdicts: 0,
    failed_runs: 0,
    points_total: 0,
    points_awarded: 0,
    points_withdrawn: 0,
    sessions: 0,
    players: 0,
    last_verdict_at: null,
  },
  ...over,
});

const worked = usage({
  judge_id: "j1",
  attachments: [
    {
      project_id: "p1",
      project_name: "Weather Widget",
      project_slug: "weather-widget",
      parent_project_id: null,
      task_id: "t1",
      task_ordinal: 0,
      task_title: "Build the widget",
    },
    {
      project_id: "p1",
      project_name: "Weather Widget",
      project_slug: "weather-widget",
      parent_project_id: null,
      task_id: "t2",
      task_ordinal: 1,
      task_title: "Switch cities",
    },
    {
      project_id: "p2",
      project_name: "Money Tracker 1/6",
      project_slug: "money-tracker-1-ledger",
      parent_project_id: "parent",
      task_id: "t3",
      task_ordinal: 1,
      task_title: "Build the ledger",
    },
  ],
  stats: {
    verdicts: 128,
    failed_runs: 3,
    points_total: 2120,
    points_awarded: 2430,
    points_withdrawn: 310,
    sessions: 40,
    players: 42,
    last_verdict_at: new Date().toISOString(),
  },
});

const data = {
  judges: [
    judge({ id: "j1", slug: "correctness", name: "Correctness" }),
    judge({ id: "j2", slug: "unused", name: "Unused" }),
  ],
  providers: [],
  pools: [],
  usage: [worked, usage({ judge_id: "j2" })],
};

/** The judge's desktop-table row. The card layout repeats the name, so pick
 *  the match that actually lives inside a `<tr>`. */
function tableRow(name: string): HTMLElement {
  const row = screen
    .getAllByText(name)
    .map((el) => el.closest("tr"))
    .find(Boolean);
  if (!row) throw new Error(`no table row for ${name}`);
  return row;
}

describe("Judges settings", () => {
  it("says where a judge runs, and how much of the record it owns", () => {
    render(JudgesPage, { data: data as never });
    const row = tableRow("Correctness");
    // Two projects, three tasks — folded, not listed three times.
    expect(within(row).getByText(/2 projects · 3 tasks/)).not.toBeNull();
    expect(within(row).getByText("128")).not.toBeNull();
    expect(within(row).getByText(/42 players/)).not.toBeNull();
    expect(within(row).getByText(/3 failed/)).not.toBeNull();
  });

  it("shows what the judge moved, and both directions when it moves both ways", () => {
    render(JudgesPage, { data: data as never });
    const row = tableRow("Correctness");
    expect(within(row).getByText("+2,120")).not.toBeNull();
    expect(within(row).getByText(/\+2,430 \/ −310/)).not.toBeNull();
  });

  it("names every project and task once the row is opened", async () => {
    render(JudgesPage, { data: data as never });
    expect(screen.queryByTestId("attachments-correctness")).toBeNull();
    await fireEvent.click(screen.getByTestId("attachments-toggle-correctness"));
    const panel = screen.getByTestId("attachments-correctness");
    expect(within(panel).getByText("Weather Widget")).not.toBeNull();
    expect(within(panel).getByText(/Build the widget/)).not.toBeNull();
    expect(within(panel).getByText(/Switch cities/)).not.toBeNull();
    expect(within(panel).getByText("Money Tracker 1/6")).not.toBeNull();
  });

  it("says plainly when a judge is attached to nothing and has never run", () => {
    render(JudgesPage, { data: data as never });
    const row = tableRow("Unused");
    expect(within(row).getByText("Not attached")).not.toBeNull();
    expect(within(row).queryByTestId("attachments-toggle-unused")).toBeNull();
    expect(within(row).getAllByText("—").length).toBe(2);
  });
});
