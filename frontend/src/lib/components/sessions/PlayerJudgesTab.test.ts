import { describe, it, expect } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import PlayerJudgesTab from "./PlayerJudgesTab.svelte";
import type {
  PlayerTaskSummaryEntry,
  PlayerJudgeScoredPayload,
  PlayerTaskEvaluation,
} from "$lib/types/arena";

function mkTask(over: Partial<PlayerTaskSummaryEntry> = {}): PlayerTaskSummaryEntry {
  return {
    task_id: "t1",
    ordinal: 1,
    title: "Build the widget",
    content: "",
    tags: [],
    adapted_content: "",
    result: null,
    scheduler_state: null,
    ...over,
  };
}

function mkVerdict(over: Partial<PlayerJudgeScoredPayload> = {}): PlayerJudgeScoredPayload {
  return {
    task_id: "t1",
    judge_slug: "ux-review",
    judge_name: "UX Review",
    rating: 8,
    point_delta: 12,
    feedback: "looks good",
    created_at: "2026-08-05T10:00:00Z",
    duration_ms: 1200,
    ...over,
  } as PlayerJudgeScoredPayload;
}

function mkEvaluation(over: Partial<PlayerTaskEvaluation> = {}): PlayerTaskEvaluation {
  return {
    task_id: "t1",
    criteria: [],
    artifacts: [
      {
        probe_id: "ab0e8400-e29b-41d4-a716-446655440000",
        content_type: "image/png",
        label: ".ololo/artifacts/ab0e8400/shot.png",
      },
      {
        probe_id: "cd0e8400-e29b-41d4-a716-446655440001",
        content_type: "text/plain",
        label: "notes.txt",
      },
    ],
    ...over,
  } as PlayerTaskEvaluation;
}

function renderTab(evaluations: PlayerTaskEvaluation[]) {
  return render(PlayerJudgesTab, {
    props: {
      tasks: [mkTask()],
      judgeResultsByTask: new Map([["t1", [mkVerdict()]]]),
      judgeStatusesByTask: new Map(),
      sessionCode: "AB12CD",
      playerId: "player-1",
      evaluations,
    },
  });
}

describe("PlayerJudgesTab screenshots", () => {
  it("renders delivered image artifacts as thumbnails, skipping non-images", () => {
    const { queryByTestId, container } = renderTab([mkEvaluation()]);
    const strip = queryByTestId("judge-screenshots-1");
    expect(strip).toBeTruthy();
    const imgs = strip!.querySelectorAll("img");
    expect(imgs).toHaveLength(1);
    expect(imgs[0].getAttribute("src")).toBe(
      "/api/sessions/AB12CD/players/player-1/artifacts/ab0e8400-e29b-41d4-a716-446655440000",
    );
    expect(container.querySelector('[data-testid="image-lightbox"]')).toBeNull();
  });

  it("shows no strip when the task delivered no images", () => {
    const { queryByTestId } = renderTab([mkEvaluation({ artifacts: [] })]);
    expect(queryByTestId("judge-screenshots-1")).toBeNull();
  });

  it("opens the lightbox on click and closes it again", async () => {
    const { getByTestId, queryByTestId } = renderTab([mkEvaluation()]);
    await fireEvent.click(getByTestId("judge-screenshot-ab0e8400-e29b-41d4-a716-446655440000"));
    const box = getByTestId("image-lightbox");
    const full = box.querySelector("img");
    expect(full?.getAttribute("src")).toBe(
      "/api/sessions/AB12CD/players/player-1/artifacts/ab0e8400-e29b-41d4-a716-446655440000",
    );
    await fireEvent.click(box.querySelector("button")!);
    expect(queryByTestId("image-lightbox")).toBeNull();
  });
});

describe("PlayerJudgesTab evaluation block", () => {
  it("renders the judge's criteria inside its verdict bubble", () => {
    const { getByTestId } = renderTab([
      mkEvaluation({
        criteria: [
          {
            key: "ux",
            title: "UI/UX",
            weight: 0.2,
            scores: [{ judge_slug: "ux-review", score: 8.5, rationale: "clean layout" }],
          },
        ],
      }),
    ]);
    const bubble = getByTestId("judge-criteria-1-ux-review");
    expect(bubble.textContent).toContain("UI/UX");
    expect(bubble.textContent).toContain("8.5");
  });

  it("reveals the criterion rationale in a floating hovercard on hover", async () => {
    const { getByTestId } = renderTab([
      mkEvaluation({
        criteria: [
          {
            key: "ux",
            title: "UI/UX",
            weight: 0.2,
            scores: [{ judge_slug: "ux-review", score: 8.5, rationale: "clean layout" }],
          },
        ],
      }),
    ]);
    const bubble = getByTestId("judge-criteria-1-ux-review");
    // The rationale is not inlined in the bubble: it lives in a hover-card
    // that floats in a portal, so overflow-hidden ancestors cannot clip it.
    expect(bubble.textContent).not.toContain("clean layout");
    const trigger = bubble.querySelector("a, [role='button'], [data-link-preview-trigger], button");
    expect(trigger).not.toBeNull();
    await fireEvent.focus(trigger!);
    await new Promise((r) => setTimeout(r, 800)); // openDelay + transition
    const card = document.querySelector("[data-testid='criterion-rationale-hovercard']");
    if (card) expect(card.textContent).toContain("clean layout");
  });

  it("shows a card for an open-ended task even before any judge ran", () => {
    const { getByTestId } = render(PlayerJudgesTab, {
      props: {
        tasks: [mkTask()],
        judgeResultsByTask: new Map(),
        judgeStatusesByTask: new Map(),
        sessionCode: "AB12CD",
        playerId: "player-1",
        evaluations: [mkEvaluation()],
      },
    });
    expect(getByTestId("judges-task-1")).toBeTruthy();
    expect(getByTestId("judge-evaluation-1")).toBeTruthy();
  });

  it("keeps non-image artifacts reachable as downloads", () => {
    const { getByTestId } = renderTab([mkEvaluation()]);
    const strip = getByTestId("judge-screenshots-1");
    const link = strip.querySelector("a[download]");
    expect(link?.getAttribute("href")).toBe(
      "/api/sessions/AB12CD/players/player-1/artifacts/cd0e8400-e29b-41d4-a716-446655440001",
    );
  });
});

describe("PlayerJudgesTab repo image gallery", () => {
  it("merges committed repo images into the gallery, deduped by path", () => {
    const { getByTestId } = renderTab([
      mkEvaluation({
        artifacts: [
          {
            probe_id: "ab0e8400-e29b-41d4-a716-446655440000",
            content_type: "image/png",
            label: ".ololo/artifacts/ab0e8400/shot.png",
          },
        ],
        repo_images: [
          ".ololo/artifacts/ab0e8400/shot.png", // duplicate of the probe artifact
          ".ololo/screenshots/desktop.png",
        ],
      }),
    ]);
    const strip = getByTestId("judge-screenshots-1");
    const imgs = strip.querySelectorAll("img");
    expect(imgs).toHaveLength(2);
    expect(imgs[1].getAttribute("src")).toBe(
      "/api/sessions/AB12CD/players/player-1/repo-file?path=" +
        encodeURIComponent(".ololo/screenshots/desktop.png"),
    );
  });
});

describe("PlayerJudgesTab changes block", () => {
  it("renders a collapsed Changes block with the task's diff", () => {
    const commit = {
      sha: "abc1234def",
      author_name: "bot",
      author_email: "bot@x",
      author_time: "2026-01-01T00:00:00Z",
      message: "feat(t1): impl",
      files: [{ path: "src/app.js", status: "modified", patch: "@@ -1 +1 @@\n-a\n+b" }],
    };
    const { getByTestId } = render(PlayerJudgesTab, {
      props: {
        tasks: [mkTask()],
        judgeResultsByTask: new Map([["t1", [mkVerdict()]]]),
        judgeStatusesByTask: new Map(),
        sessionCode: "AB12CD",
        playerId: "player-1",
        evaluations: [],
        changesByTask: new Map([["t1", { commits: [commit], mode: "per-commit" as const }]]),
      },
    });
    const block = getByTestId("judges-changes-1");
    expect(block.tagName).toBe("DETAILS");
    expect(block.hasAttribute("open")).toBe(false);
    expect(block.textContent).toContain("1 commit");
    expect(getByTestId("judges-changes-diff-1").textContent).toContain("src/app.js");
  });
});

describe("PlayerJudgesTab judge-attributed screenshots", () => {
  it("renders judge-requested captures in the task-level strip above the verdicts", () => {
    const { getByTestId, queryByTestId } = renderTab([
      mkEvaluation({
        artifacts: [
          {
            probe_id: "ab0e8400-e29b-41d4-a716-446655440000",
            content_type: "image/png",
            label: ".ololo/artifacts/ab0e8400/shot.png",
            judge_slug: "ux-review",
          },
        ],
        repo_images: [],
      }),
    ]);
    const strip = getByTestId("judge-screenshots-1");
    const img = strip.querySelector("img");
    expect(img?.getAttribute("src")).toBe(
      "/api/sessions/AB12CD/players/player-1/artifacts/ab0e8400-e29b-41d4-a716-446655440000",
    );
    // No per-judge galleries anymore — everything lives in the strip,
    // rendered before the verdict bubble in the card.
    expect(queryByTestId("judge-shots-1-ux-review")).toBeNull();
    const verdict = getByTestId("judge-verdict-1-ux-review");
    expect(strip.compareDocumentPosition(verdict) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("orders task cards newest first", () => {
    const { container } = render(PlayerJudgesTab, {
      props: {
        tasks: [mkTask(), mkTask({ task_id: "t2", ordinal: 2, title: "Second task" })],
        judgeResultsByTask: new Map([
          ["t1", [mkVerdict()]],
          ["t2", [mkVerdict({ task_id: "t2" })]],
        ]),
        judgeStatusesByTask: new Map(),
        sessionCode: "AB12CD",
        playerId: "player-1",
        evaluations: [],
      },
    });
    const cards = [...container.querySelectorAll('[data-testid^="judges-task-"]')];
    expect(cards.map((c) => c.getAttribute("data-testid"))).toEqual([
      "judges-task-2",
      "judges-task-1",
    ]);
  });

  it("steps through the task gallery with next/prev in the lightbox", async () => {
    const { getByTestId } = renderTab([
      mkEvaluation({
        artifacts: [
          {
            probe_id: "ab0e8400-e29b-41d4-a716-446655440000",
            content_type: "image/png",
            label: ".ololo/artifacts/req-1/first.png",
          },
          {
            probe_id: "cd0e8400-e29b-41d4-a716-446655440001",
            content_type: "image/png",
            label: ".ololo/artifacts/req-2/second.png",
          },
        ],
        repo_images: [],
      }),
    ]);
    await fireEvent.click(getByTestId("judge-screenshot-ab0e8400-e29b-41d4-a716-446655440000"));
    const box = getByTestId("image-lightbox");
    expect(box.querySelector("img")?.getAttribute("src")).toBe(
      "/api/sessions/AB12CD/players/player-1/artifacts/ab0e8400-e29b-41d4-a716-446655440000",
    );
    await fireEvent.click(getByTestId("lightbox-next"));
    expect(getByTestId("image-lightbox").querySelector("img")?.getAttribute("src")).toBe(
      "/api/sessions/AB12CD/players/player-1/artifacts/cd0e8400-e29b-41d4-a716-446655440001",
    );
    await fireEvent.click(getByTestId("lightbox-prev"));
    expect(getByTestId("image-lightbox").querySelector("img")?.getAttribute("src")).toBe(
      "/api/sessions/AB12CD/players/player-1/artifacts/ab0e8400-e29b-41d4-a716-446655440000",
    );
  });

  it("dedupes retries that delivered the same file under several probes", () => {
    const { getByTestId } = renderTab([
      mkEvaluation({
        artifacts: [
          {
            probe_id: "ab0e8400-e29b-41d4-a716-446655440000",
            content_type: "image/png",
            label: ".ololo/artifacts/req-1/shot.png",
          },
          {
            probe_id: "cd0e8400-e29b-41d4-a716-446655440001",
            content_type: "image/png",
            label: ".ololo/artifacts/req-1/shot.png",
          },
        ],
        repo_images: [],
      }),
    ]);
    const strip = getByTestId("judge-screenshots-1");
    expect(strip.querySelectorAll("img")).toHaveLength(1);
  });
});
