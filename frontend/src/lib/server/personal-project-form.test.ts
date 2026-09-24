import { describe, it, expect } from "vitest";
import { ApiError } from "$lib/api";
import { parsePersonalForm, personalFailure } from "./personal-project-form";

function form(fields: Record<string, string>): FormData {
  const data = new FormData();
  for (const [k, v] of Object.entries(fields)) data.set(k, v);
  return data;
}

describe("parsePersonalForm", () => {
  it("builds the request from the fields and the JSON map", () => {
    const { request, values } = parsePersonalForm(
      form({
        name: "  CSV export ",
        description: " Add a CSV export. ",
        tasks_json: JSON.stringify([
          { title: " Endpoint ", description: " GET /x.csv " },
          { title: "Button", description: "" },
          { title: "   ", description: "dropped: no title" },
        ]),
        judges_json: JSON.stringify(["correctness", "code-quality"]),
        session_duration_secs: "3600",
      }),
    );
    expect(request).toEqual({
      name: "CSV export",
      description: "Add a CSV export.",
      tasks: [{ title: "Endpoint", description: "GET /x.csv" }, { title: "Button" }],
      judges: ["correctness", "code-quality"],
      session_duration_secs: 3600,
      public: true,
      repo_url: "",
      repo_ref: "",
    });
    expect(values.tasks).toHaveLength(2);
  });

  it("carries a task's own panel, empty included, and nothing for the rest", () => {
    const { request, values } = parsePersonalForm(
      form({
        description: "d",
        tasks_json: JSON.stringify([
          { title: "Endpoint", description: "", judges: ["test-quality", 7] },
          { title: "Button", description: "" },
          { title: "Docs", description: "", judges: [] },
          { title: "Noise", description: "", judges: "correctness" },
        ]),
        judges_json: JSON.stringify(["correctness"]),
      }),
    );
    expect(request.tasks).toEqual([
      { title: "Endpoint", judges: ["test-quality", "7"] },
      { title: "Button" },
      { title: "Docs", judges: [] },
      { title: "Noise" },
    ]);
    expect(values.tasks[0].judges).toEqual(["test-quality", "7"]);
    expect("judges" in values.tasks[1]).toBe(false);
  });

  it("leaves out what the user left empty and survives garbage", () => {
    const { request } = parsePersonalForm(
      form({ description: "d", tasks_json: "{not json", judges_json: "null" }),
    );
    expect(request).toEqual({
      description: "d",
      tasks: [],
      judges: [],
      public: true,
      repo_url: "",
      repo_ref: "",
    });
  });

  it("carries the repository, and no ref without one", () => {
    const withRepo = parsePersonalForm(
      form({ description: "d", repo_url: " git@github.com:me/app.git ", repo_ref: " dev " }),
    );
    expect(withRepo.request.repo_url).toBe("git@github.com:me/app.git");
    expect(withRepo.request.repo_ref).toBe("dev");
    const none = parsePersonalForm(form({ description: "d", repo_url: "  ", repo_ref: "dev" }));
    expect(none.request.repo_url).toBe("");
    expect(none.request.repo_ref).toBe("");
  });

  it("keeps a project private only when the form says so", () => {
    expect(parsePersonalForm(form({ description: "d", public: "false" })).request.public).toBe(
      false,
    );
    expect(parsePersonalForm(form({ description: "d", public: "true" })).request.public).toBe(true);
    expect(parsePersonalForm(form({ description: "d", public: "maybe" })).values.public).toBe(true);
  });
});

describe("personalFailure", () => {
  it("carries the field and detail the server named", () => {
    const err = new ApiError(422, "invalid_personal_project", {
      error: "invalid_personal_project",
      field: "tasks",
      detail: "a navigation map holds at most 10 tasks",
    });
    const values = {
      name: "",
      description: "d",
      tasks: [],
      judges: [],
      session_duration_secs: 0,
      public: true,
      repo_url: "",
      repo_ref: "",
    };
    const result = personalFailure(err, values);
    expect(result.status).toBe(422);
    expect(result.data).toEqual({
      error: "invalid_personal_project",
      field: "tasks",
      detail: "a navigation map holds at most 10 tasks",
      values,
    });
  });

  it("names the creation switch", () => {
    const err = new ApiError(403, "project creation is currently restricted to administrators", {});
    const result = personalFailure(err, {
      name: "",
      description: "d",
      tasks: [],
      judges: [],
      session_duration_secs: 0,
      public: true,
      repo_url: "",
      repo_ref: "",
    });
    expect(result.data.error).toBe("creation_restricted");
  });
});
