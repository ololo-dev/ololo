import { describe, it, expect } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/svelte";
import SessionReportPanel from "./SessionReportPanel.svelte";
import type {
  PlayerJudgeScoredPayload,
  PlayerJudgeStatusPayload,
  PlayerSessionReport,
  PlayerTaskEvaluation,
  PlayerTaskSummaryEntry,
} from "$lib/types/arena";

function task(over: Partial<PlayerTaskSummaryEntry> & { task_id: string; ordinal: number }) {
  return {
    title: `Task ${over.ordinal}`,
    revealed_at: "2026-08-20T17:00:00Z",
    points: 10,
    bonus_points: 0,
    ...over,
  } as PlayerTaskSummaryEntry;
}

const report: PlayerSessionReport = {
  judge_name: "The Debrief",
  judge_slug: "general",
  markdown: "{}",
  created_at: "2026-08-20T18:00:00Z",
  document: {
    built: {
      brief: "A SQL engine that filters, sorts and aggregates.",
      tasks: [{ ordinal: 1, note: "column projection" }],
    },
    friction: [
      { ordinal: 3, what_happened: "the checks failed twice before passing", why: "type handling" },
    ],
    judges: [
      { judge: "Architecture", good: "clear module split", improve: "SELECT is one big function" },
      { judge: "Tests", good: "the suite runs", improve: null },
    ],
    improve: ["Write a test per operator before submitting"],
  },
};

function result(status: string) {
  return {
    status,
    submitted_answer: null,
    correct_answer: null,
    score_delta: 0,
    evaluated_at: "2026-08-20T17:30:00Z",
  };
}

const tasks = [
  task({ task_id: "t1", ordinal: 1, title: "Projection", result: result("completed") }),
  task({ task_id: "t3", ordinal: 3, title: "Comparisons", result: result("failed") }),
] as PlayerTaskSummaryEntry[];

const base = {
  report,
  sessionFinished: true,
  tasks,
  probesByTask: new Map(),
  judgeResultsByTask: new Map(),
  evaluationsByTask: new Map<string, PlayerTaskEvaluation>(),
  sessionCode: "UXC2VX",
  playerId: "p1",
  score: 120,
  rank: 1,
  totalTasks: 4,
};

describe("SessionReportPanel", () => {
  it("opens with the same summary card the chat closes with", () => {
    render(SessionReportPanel, base);
    const card = screen.getByTestId("report-summary");
    // The score and the tasks tile come from the record, not the report text.
    expect(within(card).getByText("120")).not.toBeNull();
    expect(within(card).getByText("1/4")).not.toBeNull();
  });

  it("lists the tasks that actually passed, with the judge's note", () => {
    render(SessionReportPanel, base);
    const built = screen.getByTestId("report-built");
    expect(within(built).getByText(/filters, sorts and aggregates/)).not.toBeNull();
    const entry = within(built).getByTestId("report-built-task-1");
    expect(entry.textContent).toContain("Projection");
    expect(entry.textContent).toContain("column projection");
    // A task that never passed belongs to friction, not to what was built.
    expect(within(built).queryByText("Comparisons")).toBeNull();
  });

  it("names the tasks that fought back, without platform jargon in the heading", () => {
    render(SessionReportPanel, base);
    const friction = screen.getByTestId("report-friction");
    expect(within(friction).getByText(/Task #3 · Comparisons/)).not.toBeNull();
    expect(within(friction).getByText(/failed twice before passing/)).not.toBeNull();
    expect(within(friction).getByText("type handling")).not.toBeNull();
  });

  it("falls back to the reporter's two briefs when no verdict reached the page", () => {
    render(SessionReportPanel, base);
    const judges = screen.getByTestId("report-judges");
    expect(screen.getByRole("heading", { name: "Judges" })).not.toBeNull();
    // The documentation's table: the two briefs are columns, one row per judge.
    expect(within(judges).getByRole("columnheader", { name: "What was good" })).not.toBeNull();
    expect(
      within(judges).getByRole("columnheader", { name: "What should be improved" }),
    ).not.toBeNull();
    expect(within(judges).getAllByRole("row")).toHaveLength(3); // header + two judges
    expect(within(judges).getByText("clear module split")).not.toBeNull();
    // A judge that asked for nothing says so rather than showing an empty cell.
    expect(within(judges).getByText("Nothing this time.")).not.toBeNull();
  });

  it("closes with how to improve results", () => {
    render(SessionReportPanel, base);
    expect(screen.getByRole("heading", { name: "How to improve results" })).not.toBeNull();
    expect(screen.getByText(/Write a test per operator/)).not.toBeNull();
    // The old section names are gone.
    expect(screen.queryByRole("heading", { name: /What the panel said/ })).toBeNull();
    expect(screen.queryByRole("heading", { name: /What to do next time/ })).toBeNull();
  });

  it("shows the screenshots and screencasts the session produced", () => {
    const evaluationsByTask = new Map<string, PlayerTaskEvaluation>([
      [
        "t1",
        {
          task_id: "t1",
          criteria: [],
          artifacts: [
            {
              probe_id: "pr1",
              content_type: "image/png",
              label: ".ololo/artifacts/9f2/shot.png",
              file_count: 1,
            },
            {
              probe_id: "pr2",
              content_type: "video/webm",
              label: ".ololo/artifacts/9f2/run.webm",
              file_count: 1,
            },
          ],
        } as PlayerTaskEvaluation,
      ],
    ]);
    const { container } = render(SessionReportPanel, { ...base, evaluationsByTask });
    const strip = screen.getByTestId("report-artifacts");
    // Captioned by file name: the repo path under .ololo/artifacts says
    // nothing a reader wants.
    expect(within(strip).getByAltText("shot.png")).not.toBeNull();
    expect(container.querySelector("video[title='run.webm']")).not.toBeNull();
    expect(strip.textContent).not.toContain(".ololo/artifacts");
  });

  it("names each delivered file, rather than counting off the first one", () => {
    // One request, two files: the second is receipt-mobile.png, not
    // "receipt-desktop.png (2/2)".
    const evaluationsByTask = new Map<string, PlayerTaskEvaluation>([
      [
        "t1",
        {
          task_id: "t1",
          criteria: [],
          artifacts: [
            {
              probe_id: "pr1",
              content_type: "image/png",
              label: ".ololo/artifacts/8dd/receipt-desktop.png",
              file_count: 2,
              files: [
                ".ololo/artifacts/8dd/receipt-desktop.png",
                ".ololo/artifacts/8dd/receipt-mobile.png",
              ],
            },
          ],
        } as PlayerTaskEvaluation,
      ],
    ]);
    render(SessionReportPanel, { ...base, evaluationsByTask });
    const strip = screen.getByTestId("report-artifacts");
    expect(within(strip).getByAltText("receipt-desktop.png")).not.toBeNull();
    expect(within(strip).getByAltText("receipt-mobile.png")).not.toBeNull();
    expect(strip.textContent).not.toContain("(2/2)");
  });

  it("falls back to a position when an older row carries no file list", () => {
    const evaluationsByTask = new Map<string, PlayerTaskEvaluation>([
      [
        "t1",
        {
          task_id: "t1",
          criteria: [],
          artifacts: [
            {
              probe_id: "pr1",
              content_type: "image/png",
              label: ".ololo/artifacts/8dd/book-desktop.png",
              file_count: 2,
            },
          ],
        } as PlayerTaskEvaluation,
      ],
    ]);
    render(SessionReportPanel, { ...base, evaluationsByTask });
    const strip = screen.getByTestId("report-artifacts");
    expect(within(strip).getByAltText("book-desktop.png (1/2)")).not.toBeNull();
    expect(within(strip).getByAltText("book-desktop.png (2/2)")).not.toBeNull();
  });

  describe("the artifact gallery", () => {
    const evaluationsByTask = new Map<string, PlayerTaskEvaluation>([
      [
        "t1",
        {
          task_id: "t1",
          criteria: [],
          artifacts: [
            { probe_id: "pr1", content_type: "image/png", label: "ledger.png", file_count: 1 },
            { probe_id: "pr2", content_type: "image/png", label: "mobile.png", file_count: 1 },
            { probe_id: "pr3", content_type: "video/webm", label: "run.webm", file_count: 1 },
          ],
        } as PlayerTaskEvaluation,
      ],
    ]);
    const withShots = { ...base, evaluationsByTask };

    it("opens a capture fullscreen, at the one that was clicked", async () => {
      render(SessionReportPanel, withShots);
      expect(screen.queryByTestId("image-lightbox")).toBeNull();
      await fireEvent.click(screen.getByTestId("report-artifact-pr2"));
      const box = screen.getByTestId("image-lightbox");
      expect(within(box).getByAltText("mobile.png")).not.toBeNull();
      expect(box.textContent).toContain("2 / 2");
    });

    it("walks the gallery with the arrows, and wraps at the ends", async () => {
      render(SessionReportPanel, withShots);
      await fireEvent.click(screen.getByTestId("report-artifact-pr1"));
      await fireEvent.click(screen.getByTestId("lightbox-next"));
      expect(
        within(screen.getByTestId("image-lightbox")).getByAltText("mobile.png"),
      ).not.toBeNull();
      // Past the last image is the first again — the strip is a loop.
      await fireEvent.click(screen.getByTestId("lightbox-next"));
      expect(
        within(screen.getByTestId("image-lightbox")).getByAltText("ledger.png"),
      ).not.toBeNull();
      await fireEvent.click(screen.getByTestId("lightbox-prev"));
      expect(
        within(screen.getByTestId("image-lightbox")).getByAltText("mobile.png"),
      ).not.toBeNull();
    });

    it("steps with the arrow keys and closes on Escape", async () => {
      render(SessionReportPanel, withShots);
      await fireEvent.click(screen.getByTestId("report-artifact-pr1"));
      await fireEvent.keyDown(window, { key: "ArrowRight" });
      expect(
        within(screen.getByTestId("image-lightbox")).getByAltText("mobile.png"),
      ).not.toBeNull();
      await fireEvent.keyDown(window, { key: "Escape" });
      expect(screen.queryByTestId("image-lightbox")).toBeNull();
    });

    it("keeps a screencast out of the gallery — it has its own fullscreen", async () => {
      render(SessionReportPanel, withShots);
      expect(screen.queryByTestId("report-artifact-pr3")).toBeNull();
      await fireEvent.click(screen.getByTestId("report-artifact-pr1"));
      // Two images, not three captures.
      expect(screen.getByTestId("image-lightbox").textContent).toContain("1 / 2");
    });
  });

  it("renders a prose fallback when the model did not answer with the document", () => {
    render(SessionReportPanel, {
      ...base,
      report: { ...report, document: null, markdown: "## What you built\n\nA server." },
    });
    expect(screen.getByRole("heading", { name: "What you built" })).not.toBeNull();
    expect(screen.queryByTestId("report-judges")).toBeNull();
  });

  it("says the report is coming while the judges are still working", () => {
    render(SessionReportPanel, { ...base, report: null, judgesSettling: true });
    expect(screen.getByTestId("session-report-pending")).not.toBeNull();
  });

  it("states the absence without guessing at its cause", () => {
    render(SessionReportPanel, { ...base, report: null, judgesSettling: false });
    expect(screen.getByText("No report was written for this session.")).not.toBeNull();
  });

  describe("with the panel's own verdicts", () => {
    function verdict(over: Partial<PlayerJudgeScoredPayload>): PlayerJudgeScoredPayload {
      return {
        task_id: "t1",
        judge_slug: "architecture",
        judge_name: "Architecture",
        rating: 9,
        feedback: "Clean layering between storage and the API.",
        point_delta: 23,
        created_at: "2026-08-20T17:40:00Z",
        duration_ms: 3674,
        ...over,
      } as PlayerJudgeScoredPayload;
    }

    const judgeResultsByTask = new Map<string, PlayerJudgeScoredPayload[]>([
      [
        "t1",
        [
          verdict({}),
          verdict({
            judge_slug: "tests",
            judge_name: "Tests",
            point_delta: 4,
            feedback: "The suite runs, but nothing covers the comparison path.",
          }),
        ],
      ],
      [
        "t3",
        [
          verdict({
            task_id: "t3",
            judge_slug: "architecture",
            judge_name: "Architecture",
            point_delta: -5,
            feedback: "The comparison path duplicates the projection path.",
            created_at: "2026-08-20T17:50:00Z",
          }),
        ],
      ],
    ]);

    const withVerdicts = { ...base, judgeResultsByTask };

    it("shows each verdict under the task it scored, with what it moved", () => {
      render(SessionReportPanel, withVerdicts);
      const judges = screen.getByTestId("report-judges");
      // Two tasks were judged, so the panel groups by task.
      const first = within(judges).getByTestId("report-judges-task-1");
      const second = within(judges).getByTestId("report-judges-task-3");
      expect(first.textContent).toContain("Task #1 · Projection");
      expect(first.textContent).toContain("+27 pts"); // 23 + 4, the task's total
      expect(second.textContent).toContain("-5 pts");
      // The judge's own words, which the old table dropped entirely.
      expect(within(first).getByText(/Clean layering between storage and the API/)).not.toBeNull();
      expect(
        within(second).getByText(/comparison path duplicates the projection path/),
      ).not.toBeNull();
    });

    it("pairs the reporter's briefs onto the verdicts, in task order", () => {
      render(SessionReportPanel, withVerdicts);
      const first = screen.getByTestId("report-judges-task-1");
      // "Architecture" appears once in the document and twice in the record:
      // the note belongs to the first task that judge scored.
      expect(within(first).getByText("clear module split")).not.toBeNull();
      expect(within(first).getByText("SELECT is one big function")).not.toBeNull();
      const second = screen.getByTestId("report-judges-task-3");
      expect(within(second).queryByText("clear module split")).toBeNull();
    });

    it("names a judge that produced no verdict, and why", () => {
      const judgeStatusesByTask = new Map<string, PlayerJudgeStatusPayload[]>([
        [
          "t1",
          [
            {
              task_id: "t1",
              judge_slug: "ux-review",
              judge_name: "UX Review",
              status: "failed",
              error: "no screenshot was delivered",
            } as PlayerJudgeStatusPayload,
          ],
        ],
      ]);
      render(SessionReportPanel, { ...withVerdicts, judgeStatusesByTask });
      const note = screen.getByTestId("report-judges-unscored");
      expect(note.textContent).toContain("UX Review");
      expect(note.textContent).toContain("no screenshot was delivered");
    });

    it("lays the criteria sheet out with its weights and the lines behind it", () => {
      const evaluationsByTask = new Map<string, PlayerTaskEvaluation>([
        [
          "t1",
          {
            task_id: "t1",
            criteria: [
              {
                key: "product",
                title: "Product",
                weight: 0.25,
                scores: [{ judge_slug: "architecture", score: 8.5, rationale: "every flow works" }],
              },
              {
                key: "tests",
                title: "Tests",
                weight: 0.1,
                scores: [{ judge_slug: "tests", score: null, rationale: "nothing to run" }],
              },
            ],
          } as PlayerTaskEvaluation,
        ],
      ]);
      render(SessionReportPanel, { ...withVerdicts, evaluationsByTask });
      const sheet = screen.getByTestId("report-scorecard");
      expect(screen.getByRole("heading", { name: "The scorecard" })).not.toBeNull();
      // Heaviest criterion first, with the weight it carried and who scored it.
      const rows = within(sheet).getAllByTestId("report-scorecard-row");
      expect(rows).toHaveLength(2);
      expect(rows[0].textContent).toContain("Product");
      expect(rows[0].textContent).toContain("25%");
      expect(rows[0].textContent).toContain("8.5");
      expect(rows[0].textContent).toContain("Architecture");
      expect(rows[0].textContent).toContain("every flow works");
      // A criterion the judge could not assess says so rather than showing 0.
      expect(rows[1].textContent).toContain("—");
    });

    it("renders a rationale as markdown, without eating its glob paths", () => {
      const evaluationsByTask = new Map<string, PlayerTaskEvaluation>([
        [
          "t1",
          {
            task_id: "t1",
            criteria: [
              {
                key: "architecture",
                title: "Architecture",
                weight: 0.1,
                scores: [
                  {
                    judge_slug: "architecture",
                    score: 9,
                    rationale:
                      "Domain (src/domain/*.ts) is isolated from `src/storage`, and **nothing** leaks.",
                  },
                ],
              },
            ],
          } as PlayerTaskEvaluation,
        ],
      ]);
      const { container } = render(SessionReportPanel, { ...withVerdicts, evaluationsByTask });
      const row = screen.getByTestId("report-scorecard-row");
      // Markdown did its job: the code span and the bold are elements now.
      expect(row.querySelector("code")?.textContent).toBe("src/storage");
      expect(row.querySelector("strong")?.textContent).toBe("nothing");
      // ...and the glob survived as written rather than turning into emphasis.
      expect(row.textContent).toContain("src/domain/*.ts");
      expect(container.querySelector("em")).toBeNull();
    });

    it("keeps the reporter's table out of the way once real verdicts exist", () => {
      render(SessionReportPanel, withVerdicts);
      const judges = screen.getByTestId("report-judges");
      expect(within(judges).queryByRole("columnheader", { name: "What was good" })).toBeNull();
    });
  });

  it("survives a report that names the same task twice in friction", () => {
    // The friction list is the model's own: two rough patches on one task is
    // legitimate content, and BZU2NI produced exactly that — keyed by task
    // ordinal, the whole player page threw each_key_duplicate and never
    // opened. LLM output must never be a render key.
    const doubled: PlayerSessionReport = {
      ...report,
      document: {
        ...report.document!,
        friction: [
          { ordinal: 1, what_happened: "the balance check failed", why: null },
          { ordinal: 1, what_happened: "the totals drifted after the edit", why: null },
        ],
        judges: [
          { judge: "Architecture", good: "clear split", improve: null },
          { judge: "Architecture", good: "repeated by the model", improve: null },
        ],
      },
    };
    render(SessionReportPanel, { ...base, report: doubled });

    const friction = screen.getByTestId("report-friction");
    expect(within(friction).getByText(/the balance check failed/)).not.toBeNull();
    expect(within(friction).getByText(/the totals drifted after the edit/)).not.toBeNull();
  });
});
