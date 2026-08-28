import { describe, it, expectTypeOf } from "vitest";
import type {
  PlayerSummary,
  PlayerFrame,
  PlayerCriterionScore,
  PlayerSnapshotPayload,
  PlayerTaskEvaluation,
  PlayerTaskSummaryEntry,
  ArenaFrame,
  TaskRevealedPayload,
  ScoreRankUpdatedPayload,
} from "./arena.js";

describe("PlayerFrame types", () => {
  it("PlayerFrame discriminates on type", () => {
    const frame: PlayerFrame = { type: "player_error", seq: 1, message: "oops" };
    expectTypeOf(frame).toMatchTypeOf<PlayerFrame>();
  });

  it("PlayerFrame includes task_revealed variant", () => {
    const frame: PlayerFrame = {
      type: "task_revealed",
      seq: 1,
      task: {
        task_id: "t1",
        ordinal: 1,
        title: "T",
        content: "",
        tags: [],
        adapted_content: "",
        result: null,
        scheduler_state: null,
      },
      total_tasks: 5,
    };
    expectTypeOf(frame).toMatchTypeOf<PlayerFrame>();
  });

  it("PlayerFrame includes score_rank_updated variant", () => {
    const frame: PlayerFrame = { type: "score_rank_updated", seq: 1, score: 10, rank: 2 };
    expectTypeOf(frame).toMatchTypeOf<PlayerFrame>();
  });

  it("TaskRevealedPayload has required fields", () => {
    expectTypeOf<TaskRevealedPayload>().toHaveProperty("seq");
    expectTypeOf<TaskRevealedPayload>().toHaveProperty("task");
    expectTypeOf<TaskRevealedPayload>().toHaveProperty("total_tasks");
  });

  it("ScoreRankUpdatedPayload has required fields", () => {
    expectTypeOf<ScoreRankUpdatedPayload>().toHaveProperty("seq");
    expectTypeOf<ScoreRankUpdatedPayload>().toHaveProperty("score");
    expectTypeOf<ScoreRankUpdatedPayload>().toHaveProperty("rank");
  });

  it("PlayerTaskEvaluation carries criteria, todo, artifacts", () => {
    expectTypeOf<PlayerTaskEvaluation>().toHaveProperty("task_id");
    expectTypeOf<PlayerTaskEvaluation>().toHaveProperty("criteria");
    expectTypeOf<PlayerTaskEvaluation>().toHaveProperty("todo");
    expectTypeOf<PlayerTaskEvaluation>().toHaveProperty("pending_artifacts");
    expectTypeOf<PlayerTaskEvaluation>().toHaveProperty("measurements");
    expectTypeOf<PlayerTaskEvaluation>().toHaveProperty("artifacts");
    // A criterion score may be null — "cannot assess" is a first-class state.
    expectTypeOf<PlayerCriterionScore["score"]>().toEqualTypeOf<number | null | undefined>();
  });

  it("PlayerSnapshotPayload has required fields", () => {
    expectTypeOf<PlayerSnapshotPayload>().toHaveProperty("player_id");
    expectTypeOf<PlayerSnapshotPayload>().toHaveProperty("tasks");
    expectTypeOf<PlayerSnapshotPayload>().toHaveProperty("probes");
    expectTypeOf<PlayerSnapshotPayload>().toHaveProperty("last_seq");
    expectTypeOf<PlayerSnapshotPayload>().toHaveProperty("total_tasks");
  });

  it("PlayerSummary has player_id", () => {
    expectTypeOf<PlayerSummary>().toHaveProperty("player_id");
  });

  it("ArenaFrame includes user_players_snapshot", () => {
    const frame: ArenaFrame = { type: "user_players_snapshot", players: [] };
    expectTypeOf(frame).toMatchTypeOf<ArenaFrame>();
  });
});
