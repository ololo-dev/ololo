import { describe, it, expect } from "vitest";
import { render } from "@testing-library/svelte";
import CriteriaTrendChart from "./CriteriaTrendChart.svelte";
import type { PlayerTaskEvaluation, PlayerTaskSummaryEntry } from "$lib/types/arena";

function task(ordinal: number): PlayerTaskSummaryEntry {
  return {
    task_id: `t${ordinal}`,
    ordinal,
    title: `Task ${ordinal}`,
    content: "",
    tags: [],
    adapted_content: "",
    result: null,
    scheduler_state: null,
  };
}

/** An evaluation whose criteria carry one score per named judge. */
function evaluation(
  taskId: string,
  criteria: { key: string; title: string; scores: (number | null)[] }[],
): PlayerTaskEvaluation {
  return {
    task_id: taskId,
    criteria: criteria.map((c) => ({
      key: c.key,
      title: c.title,
      weight: 0.1,
      scores: c.scores.map((score, i) => ({
        judge_slug: `judge-${i}`,
        score,
        rationale: "because",
      })),
    })),
  };
}

/** The block ships folded; open it the way a reader would. */
async function expand(getByTestId: (id: string) => HTMLElement) {
  getByTestId("criteria-toggle").click();
  await new Promise((r) => setTimeout(r, 0));
}

describe("CriteriaTrendChart", () => {
  it("ships folded, and opens to the table view", async () => {
    const { getByTestId, queryByTestId } = render(CriteriaTrendChart, {
      props: {
        tasks: [task(0), task(1)],
        evaluations: [
          evaluation("t0", [{ key: "ux", title: "UI/UX", scores: [8] }]),
          evaluation("t1", [{ key: "ux", title: "UI/UX", scores: [9] }]),
        ],
      },
    });
    // Folded: the heading is there, the content is not.
    expect(getByTestId("criteria-trend").textContent).toContain("Criteria across tasks");
    expect(queryByTestId("criteria-table")).toBeNull();
    expect(queryByTestId("criteria-facet-ux")).toBeNull();
    expect(getByTestId("criteria-toggle").getAttribute("aria-expanded")).toBe("false");

    await expand(getByTestId);
    // Table is the default view — the numbers outright.
    expect(getByTestId("criteria-table").textContent).toContain("UI/UX");
    expect(queryByTestId("criteria-facet-ux")).toBeNull();
  });
  it("renders nothing until two tasks carry criteria", () => {
    const { queryByTestId } = render(CriteriaTrendChart, {
      props: {
        tasks: [task(0), task(1)],
        evaluations: [evaluation("t0", [{ key: "ux", title: "UI/UX", scores: [8] }])],
      },
    });
    // A trend needs two points; one scored task is not a trend.
    expect(queryByTestId("criteria-trend")).toBeNull();
  });

  it("renders one facet per criterion, averaging the judges", async () => {
    const { getByTestId } = render(CriteriaTrendChart, {
      props: {
        tasks: [task(0), task(1)],
        evaluations: [
          evaluation("t0", [
            { key: "ux", title: "UI/UX", scores: [8, 6] },
            { key: "tests", title: "Tests", scores: [5] },
          ]),
          evaluation("t1", [
            { key: "ux", title: "UI/UX", scores: [9, 9] },
            { key: "tests", title: "Tests", scores: [4] },
          ]),
        ],
      },
    });
    await expand(getByTestId);
    getByTestId("criteria-view-chart").click();
    await new Promise((r) => setTimeout(r, 0));
    const ux = getByTestId("criteria-facet-ux");
    // Latest value is the direct label: (9 + 9) / 2 = 9.0.
    expect(ux.textContent).toContain("UI/UX");
    expect(ux.textContent).toContain("9.0");
    // Delta from the first scored task: 9.0 − 7.0 = +2.0, up.
    expect(getByTestId("criteria-delta-ux").textContent).toContain("▲+2.0");
    // A criterion that slipped reads as a fall, spelled by the glyph too.
    expect(getByTestId("criteria-delta-tests").textContent).toContain("▼−1.0");
  });

  it("breaks the line at an unscored task instead of inventing a value", async () => {
    const { getByTestId } = render(CriteriaTrendChart, {
      props: {
        tasks: [task(0), task(1), task(2)],
        evaluations: [
          evaluation("t0", [{ key: "ux", title: "UI/UX", scores: [8] }]),
          // Task 1: the judge could not assess it — score null.
          evaluation("t1", [{ key: "ux", title: "UI/UX", scores: [null] }]),
          evaluation("t2", [{ key: "ux", title: "UI/UX", scores: [6] }]),
        ],
      },
    });
    await expand(getByTestId);
    getByTestId("criteria-view-chart").click();
    await new Promise((r) => setTimeout(r, 0));
    const facet = getByTestId("criteria-facet-ux");
    // Two isolated points, so two round dots and no connecting path: a gap
    // must never be drawn through.
    const paths = [...facet.querySelectorAll("path")];
    const lines = paths
      .filter((p) => (p.getAttribute("d") ?? "").includes("L"))
      .filter((p) => {
        const d = p.getAttribute("d") ?? "";
        const [, from, to] = d.match(/M ([\d.]+) [\d.]+ L ([\d.]+)/) ?? [];
        return from !== to; // a zero-length path is a dot, not a line
      });
    expect(lines.length).toBe(0);
  });

  it("offers a table view carrying every number", async () => {
    const { getByTestId, queryByTestId } = render(CriteriaTrendChart, {
      props: {
        tasks: [task(0), task(1)],
        evaluations: [
          evaluation("t0", [{ key: "ux", title: "UI/UX", scores: [8] }]),
          evaluation("t1", [{ key: "ux", title: "UI/UX", scores: [9] }]),
        ],
      },
    });
    await expand(getByTestId);
    const table = getByTestId("criteria-table");
    expect(table.textContent).toContain("UI/UX");
    expect(table.textContent).toContain("8.0");
    expect(table.textContent).toContain("9.0");
    expect(queryByTestId("criteria-facet-ux")).toBeNull();
  });

  it("keeps facets on one shared scale so two facets can be compared", async () => {
    const { getByTestId } = render(CriteriaTrendChart, {
      props: {
        tasks: [task(0), task(1)],
        evaluations: [
          evaluation("t0", [
            { key: "low", title: "Low", scores: [1] },
            { key: "high", title: "High", scores: [9] },
          ]),
          evaluation("t1", [
            { key: "low", title: "Low", scores: [1] },
            { key: "high", title: "High", scores: [9] },
          ]),
        ],
      },
    });
    await expand(getByTestId);
    getByTestId("criteria-view-chart").click();
    await new Promise((r) => setTimeout(r, 0));
    const yOf = (testid: string) => {
      const d =
        [...getByTestId(testid).querySelectorAll("path")]
          .map((p) => p.getAttribute("d") ?? "")
          .find((s) => s.includes("L") && !/M ([\d.]+) ([\d.]+) L \1 \2/.test(s)) ?? "";
      return Number(d.match(/M [\d.]+ ([\d.]+)/)?.[1] ?? NaN);
    };
    // Same 0–10 domain for both: the 9 must sit visibly higher (smaller y)
    // than the 1. A per-facet auto-scale would flatten both to the same line
    // and quietly lie.
    expect(yOf("criteria-facet-high")).toBeLessThan(yOf("criteria-facet-low"));
  });
});
