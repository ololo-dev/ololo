import { describe, it, expect, afterEach } from "vitest";
import { render, screen, cleanup } from "@testing-library/svelte";
import ActivityLog from "./ActivityLog.svelte";
import type { ActivityEvent } from "$lib/types/arena";

function artifactEvent(
  overrides: Partial<NonNullable<ActivityEvent["detail"]>> = {},
): ActivityEvent {
  return {
    kind: "artifact_received",
    player_id: "p1",
    player_display_name: "Anod",
    task_id: "t1",
    task_ordinal: 0,
    task_title: "Build the weather widget",
    judge_name: null,
    point_delta: null,
    detail: {
      probe_id: "probe-1",
      path: ".ololo/artifacts/probe-1/shot.png",
      size: 204306,
      content_type: "image/png",
      within_cap: true,
      ...overrides,
    },
    timestamp: "2026-08-08T13:10:00Z",
    version: 7,
  };
}

describe("ActivityLog artifacts", () => {
  afterEach(() => cleanup());

  it("renders_a_received_screenshot_inline", () => {
    render(ActivityLog, { props: { events: [artifactEvent()], sessionId: "sess-1" } });
    const img = screen.getByTestId("artifact-image").querySelector("img");
    expect(img?.getAttribute("src")).toBe("/api/sessions/sess-1/artifacts/probe-1");
    expect(screen.getByText("shot.png")).toBeTruthy();
    expect(screen.getByText("200 KB")).toBeTruthy();
  });

  it("hides_zero_byte_artifact_deliveries", () => {
    render(ActivityLog, { props: { events: [artifactEvent({ size: 0 })], sessionId: "sess-1" } });
    expect(screen.queryByTestId("artifact-image")).toBeNull();
    expect(screen.getByText("No activity yet.")).toBeTruthy();
  });

  it("opens_the_fullscreen_preview_and_steps_between_artifacts", async () => {
    const { fireEvent } = await import("@testing-library/svelte");
    const second = artifactEvent({
      probe_id: "probe-2",
      path: ".ololo/artifacts/probe-2/next.png",
    });
    second.timestamp = "2026-08-08T13:11:00Z";
    render(ActivityLog, {
      props: { events: [artifactEvent(), second], sessionId: "sess-1" },
    });
    // Newest first: the first thumbnail button belongs to probe-2.
    const buttons = screen
      .getAllByTestId("artifact-image")
      .flatMap((strip) => [...strip.querySelectorAll("button")]);
    expect(buttons).toHaveLength(2);
    await fireEvent.click(buttons[0]);
    const box = screen.getByTestId("image-lightbox");
    expect(box.querySelector("img")?.getAttribute("src")).toBe(
      "/api/sessions/sess-1/artifacts/probe-2",
    );
    await fireEvent.click(screen.getByTestId("lightbox-next"));
    expect(screen.getByTestId("image-lightbox").querySelector("img")?.getAttribute("src")).toBe(
      "/api/sessions/sess-1/artifacts/probe-1",
    );
    await fireEvent.click(screen.getByTestId("lightbox-prev"));
    expect(screen.getByTestId("image-lightbox").querySelector("img")?.getAttribute("src")).toBe(
      "/api/sessions/sess-1/artifacts/probe-2",
    );
  });

  it("renders_every_file_of_a_multi_shot_delivery", () => {
    render(ActivityLog, {
      props: {
        events: [
          artifactEvent({
            files: [
              { path: ".ololo/artifacts/probe-1/desktop.png", size: 1000 },
              { path: ".ololo/artifacts/probe-1/mobile.png", size: 2000 },
              { path: ".ololo/artifacts/probe-1/states.png", size: 3000 },
            ],
          }),
        ],
        sessionId: "sess-1",
      },
    });
    const imgs = screen.getByTestId("artifact-image").querySelectorAll("img");
    expect(imgs.length).toBe(3);
    expect(imgs[0].getAttribute("src")).toBe("/api/sessions/sess-1/artifacts/probe-1");
    expect(imgs[1].getAttribute("src")).toBe("/api/sessions/sess-1/artifacts/probe-1?i=1");
    expect(imgs[2].getAttribute("src")).toBe("/api/sessions/sess-1/artifacts/probe-1?i=2");
    expect(screen.getByText("3 files")).toBeTruthy();
    expect(screen.getByText("6 KB")).toBeTruthy();
  });

  it("renders_a_screencast_as_a_video_player", () => {
    render(ActivityLog, {
      props: {
        events: [
          artifactEvent({ path: ".ololo/artifacts/probe-1/walk.webm", content_type: "video/webm" }),
        ],
        sessionId: "sess-1",
      },
    });
    const video = screen.getByTestId("artifact-video").querySelector("video");
    expect(video?.getAttribute("src")).toBe("/api/sessions/sess-1/artifacts/probe-1");
    expect(video?.hasAttribute("controls")).toBe(true);
  });

  it("notes_other_artifact_types_as_an_info_line", () => {
    render(ActivityLog, {
      props: {
        events: [
          artifactEvent({
            path: ".ololo/artifacts/probe-1/coverage.txt",
            content_type: "text/plain",
          }),
        ],
        sessionId: "sess-1",
      },
    });
    const info = screen.getByTestId("artifact-info");
    expect(info.textContent).toContain("text/plain");
    expect(info.textContent).toContain("coverage.txt");
    expect(info.textContent).toContain("download");
  });

  it("falls_back_to_text_without_a_session_id", () => {
    render(ActivityLog, { props: { events: [artifactEvent()] } });
    expect(screen.queryByTestId("artifact-image")).toBeNull();
    expect(screen.getByTestId("artifact-info").textContent).toContain("image/png");
  });
});

function scoredEvent(
  feedback: string,
  detail: Partial<NonNullable<ActivityEvent["detail"]>> = {},
): ActivityEvent {
  return {
    kind: "task_scored",
    player_id: "p1",
    player_display_name: "Anod",
    task_id: "t1",
    task_ordinal: 0,
    task_title: "Build the core game",
    judge_name: "Correctness",
    point_delta: 34,
    detail: { feedback, ...detail },
    timestamp: "2026-08-08T13:12:00Z",
    version: 8,
  };
}

describe("ActivityLog judge comments", () => {
  afterEach(() => cleanup());

  it("shows the judge's written comment on a scored verdict", () => {
    render(ActivityLog, {
      props: { events: [scoredEvent("Solid core loop; rotation and line clears work.")] },
    });
    expect(screen.getByTestId("verdict-feedback").textContent).toContain("Solid core loop");
  });

  it("renders the verdict as markdown, not as raw syntax", () => {
    render(ActivityLog, {
      props: {
        events: [scoredEvent("The `handle_session` loop in `src/bin/server.rs` **never** breaks.")],
      },
    });
    const box = screen.getByTestId("verdict-feedback");
    expect([...box.querySelectorAll("code")].map((c) => c.textContent)).toEqual([
      "handle_session",
      "src/bin/server.rs",
    ]);
    expect(box.querySelector("strong")?.textContent).toBe("never");
    // The backticks and asterisks themselves must not reach the reader.
    expect(box.textContent).not.toContain("`");
    expect(box.textContent).not.toContain("**");
  });

  /**
   * jsdom does no layout, so the clamp measurement has to be staged: heights
   * are the only thing the component reads to decide whether the toggle is
   * worth showing.
   */
  function stageOverflow(clipped: boolean) {
    const descriptors = {
      clientHeight: { configurable: true, value: 54 },
      scrollHeight: { configurable: true, value: clipped ? 90 : 54 },
    };
    Object.defineProperties(HTMLElement.prototype, descriptors);
    return () => {
      Reflect.deleteProperty(HTMLElement.prototype, "clientHeight");
      Reflect.deleteProperty(HTMLElement.prototype, "scrollHeight");
    };
  }

  it("clamps a comment the column cannot hold and toggles it open", async () => {
    const { fireEvent } = await import("@testing-library/svelte");
    const restore = stageOverflow(true);
    try {
      render(ActivityLog, { props: { events: [scoredEvent("detail ".repeat(60))] } });
      const clamp = () => screen.getByTestId("verdict-feedback").firstElementChild;
      expect(clamp()?.classList.contains("clamped")).toBe(true);
      await fireEvent.click(screen.getByText("Show more"));
      expect(screen.getByText("Show less")).toBeTruthy();
      expect(clamp()?.classList.contains("clamped")).toBe(false);
    } finally {
      restore();
    }
  });

  /**
   * Regression: the toggle used to appear whenever the comment ran past 160
   * characters, but at the feed's ~990px three lines hold far more than that.
   * Half the verdicts on a live dashboard offered "Show more" and revealed
   * nothing when clicked.
   */
  it("offers no toggle when the whole comment already fits", () => {
    const restore = stageOverflow(false);
    try {
      render(ActivityLog, { props: { events: [scoredEvent("detail ".repeat(60))] } });
      expect(screen.getByTestId("verdict-feedback")).toBeTruthy();
      expect(screen.queryByText("Show more")).toBeNull();
    } finally {
      restore();
    }
  });
});
