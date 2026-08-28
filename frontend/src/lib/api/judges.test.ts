import { beforeEach, describe, expect, it, vi } from "vitest";
import { request } from "./core";
import {
  listJudges,
  getJudge,
  createJudge,
  updateJudge,
  deleteJudge,
  listTaskJudges,
  attachTaskJudge,
  updateTaskJudge,
  detachTaskJudge,
  runJudge,
} from "./judges";
import type {
  CreateJudgeBody,
  UpdateJudgeBody,
  AttachTaskJudgeBody,
  UpdateTaskJudgeBody,
  JudgeRunRequest,
} from "./types";

vi.mock("./core", () => ({ request: vi.fn() }));

const mockRequest = vi.mocked(request);

describe("judges api", () => {
  beforeEach(() => mockRequest.mockReset());

  it("listJudges GETs all judges", async () => {
    mockRequest.mockResolvedValue([{ id: "j1" }]);
    const res = await listJudges();
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/judges", { fetch: undefined });
    expect(res).toEqual([{ id: "j1" }]);
  });

  it("getJudge GETs by id", async () => {
    mockRequest.mockResolvedValue({ id: "j1" });
    await getJudge("j1");
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/judges/j1", { fetch: undefined });
  });

  it("createJudge POSTs the body", async () => {
    const body: CreateJudgeBody = {
      slug: "s",
      name: "N",
      description: "d",
      prompt: "p",
      rating_scale: { min: 1, max: 5, step: 1 },
    };
    mockRequest.mockResolvedValue({ id: "j1" });
    await createJudge(body);
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/judges", {
      method: "POST",
      body,
      fetch: undefined,
    });
  });

  it("updateJudge PUTs by id", async () => {
    const body: UpdateJudgeBody = { name: "N2" };
    mockRequest.mockResolvedValue({ id: "j1" });
    await updateJudge("j1", body);
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/judges/j1", {
      method: "PUT",
      body,
      fetch: undefined,
    });
  });

  it("deleteJudge DELETEs by id", async () => {
    mockRequest.mockResolvedValue(undefined);
    await deleteJudge("j1");
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/judges/j1", {
      method: "DELETE",
      fetch: undefined,
    });
  });

  it("listTaskJudges GETs task judges", async () => {
    mockRequest.mockResolvedValue([{ id: "tj1" }]);
    const res = await listTaskJudges("p1", "t1");
    expect(mockRequest).toHaveBeenCalledWith("/api/projects/p1/tasks/t1/judges", {
      fetch: undefined,
    });
    expect(res).toEqual([{ id: "tj1" }]);
  });

  it("attachTaskJudge POSTs a body", async () => {
    const body: AttachTaskJudgeBody = { judge_id: "j1", ordinal: 0 };
    mockRequest.mockResolvedValue({ id: "tj1" });
    await attachTaskJudge("p1", "t1", body);
    expect(mockRequest).toHaveBeenCalledWith("/api/projects/p1/tasks/t1/judges", {
      method: "POST",
      body,
      fetch: undefined,
    });
  });

  it("updateTaskJudge PUTs by judge id", async () => {
    const body: UpdateTaskJudgeBody = { ordinal: 2 };
    mockRequest.mockResolvedValue({ id: "tj1" });
    await updateTaskJudge("p1", "t1", "j1", body);
    expect(mockRequest).toHaveBeenCalledWith("/api/projects/p1/tasks/t1/judges/j1", {
      method: "PUT",
      body,
      fetch: undefined,
    });
  });

  it("detachTaskJudge DELETEs by judge id", async () => {
    mockRequest.mockResolvedValue(undefined);
    await detachTaskJudge("p1", "t1", "j1");
    expect(mockRequest).toHaveBeenCalledWith("/api/projects/p1/tasks/t1/judges/j1", {
      method: "DELETE",
      fetch: undefined,
    });
  });

  it("runJudge POSTs a run request", async () => {
    const req: JudgeRunRequest = { task_id: "t1", judge_id: "j1" };
    mockRequest.mockResolvedValue({
      judge_result_id: "r1",
      rating: 4,
      feedback: "",
      point_delta: 1,
      raw_output: "",
      model: "m",
    });
    await runJudge("code", "pl1", req);
    expect(mockRequest).toHaveBeenCalledWith("/api/admin/sessions/code/players/pl1/judge-runs", {
      method: "POST",
      body: req,
      fetch: undefined,
    });
  });
});
