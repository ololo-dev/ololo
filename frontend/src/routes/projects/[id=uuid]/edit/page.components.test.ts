import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { tick } from "svelte";
import ProjectPointsFields from "$lib/components/ProjectPointsFields.svelte";
import ProjectIntervalsFields from "$lib/components/ProjectIntervalsFields.svelte";
import ProjectSessionDurationField from "$lib/components/ProjectSessionDurationField.svelte";
import ProjectMemorySchemaFields from "$lib/components/ProjectMemorySchemaFields.svelte";
import EditAiForms from "./EditAiForms.svelte";

describe("ProjectPointsFields", () => {
  it("Renders four number inputs with correct field names when open", () => {
    render(ProjectPointsFields, {
      open: true,
      idPrefix: "edit",
      baseline: "task inheritance baseline",
    });
    expect(document.querySelector("input#edit-points-value")).not.toBeNull();
    expect(document.querySelector("input#edit-points-fail")).not.toBeNull();
    expect(document.querySelector("input#edit-points-no-response")).not.toBeNull();
    expect(document.querySelector("input#edit-points-completion-bonus")).not.toBeNull();
    const names = Array.from(document.querySelectorAll('input[type="number"]')).map(
      (el) => (el as HTMLInputElement).name,
    );
    expect(names).toEqual([
      "points_value",
      "points_fail",
      "points_no_response",
      "points_completion_bonus",
    ]);
  });

  it("Shows the baseline note text", () => {
    render(ProjectPointsFields, { open: true, baseline: "task inheritance baseline" });
    expect(screen.getByText(/task inheritance baseline/)).not.toBeNull();
  });

  it("Collapses the inputs until the header is toggled open", async () => {
    render(ProjectPointsFields, { open: false, idPrefix: "edit" });
    expect(document.querySelector("input#edit-points-value")).toBeNull();
    await fireEvent.click(screen.getByText(/Points defaults/));
    await tick();
    expect(document.querySelector("input#edit-points-value")).not.toBeNull();
  });

  it("Updates the bound value when the user types", async () => {
    render(ProjectPointsFields, { open: true, value: "" });
    const input = document.querySelector('input[name="points_value"]') as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "42" } });
    expect(input.value).toBe("42");
  });
});

describe("ProjectIntervalsFields", () => {
  it("Renders four number inputs with correct field names when open", () => {
    render(ProjectIntervalsFields, {
      open: true,
      idPrefix: "edit",
      baseline: "task inheritance baseline",
    });
    expect(document.querySelector("input#edit-intervals-deadline")).not.toBeNull();
    expect(document.querySelector("input#edit-intervals-min")).not.toBeNull();
    expect(document.querySelector("input#edit-intervals-increment")).not.toBeNull();
    expect(document.querySelector("input#edit-intervals-max")).not.toBeNull();
    const names = Array.from(document.querySelectorAll('input[type="number"]')).map(
      (el) => (el as HTMLInputElement).name,
    );
    expect(names).toEqual([
      "intervals_deadline_secs",
      "intervals_min_interval_secs",
      "intervals_interval_increment_secs",
      "intervals_max_interval_secs",
    ]);
  });

  it("Collapses the inputs until the header is toggled open", async () => {
    render(ProjectIntervalsFields, { open: false, idPrefix: "edit" });
    expect(document.querySelector("input#edit-intervals-deadline")).toBeNull();
    await fireEvent.click(screen.getByText(/Intervals defaults/));
    await tick();
    expect(document.querySelector("input#edit-intervals-deadline")).not.toBeNull();
  });
});

describe("ProjectSessionDurationField", () => {
  it("Renders a number input with the correct field name and bounds", () => {
    render(ProjectSessionDurationField, { value: "3600", idPrefix: "edit" });
    const input = document.querySelector("input#edit-session-duration") as HTMLInputElement;
    expect(input).not.toBeNull();
    expect(input.name).toBe("session_duration_secs");
    expect(input.type).toBe("number");
    expect(input.min).toBe("60");
    expect(input.max).toBe("86400");
    expect(input.value).toBe("3600");
  });

  it("Shows the valid-range hint text", () => {
    render(ProjectSessionDurationField, { value: "" });
    expect(screen.getByText(/60–86400, default 3600/)).not.toBeNull();
  });

  it("Reports typed values through the onchange callback", async () => {
    let latest = "";
    render(ProjectSessionDurationField, {
      value: "",
      onchange: (v: string) => (latest = v),
    });
    const input = document.querySelector('input[name="session_duration_secs"]') as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "7200" } });
    expect(input.value).toBe("7200");
    expect(latest).toBe("7200");
  });
});

describe("ProjectMemorySchemaFields", () => {
  const rowsOf = () => Array.from(document.querySelectorAll('[data-testid="memory-schema-row"]'));

  it("Serializes the declared keys into the hidden memory_schema_json field", () => {
    render(ProjectMemorySchemaFields, {
      open: true,
      rows: [
        { key: "dev", value: "npm run dev" },
        { key: "port", value: "1234" },
      ],
    });
    const hidden = document.querySelector('input[name="memory_schema_json"]') as HTMLInputElement;
    expect(JSON.parse(hidden.value)).toEqual([
      { key: "dev", value: "npm run dev" },
      { key: "port", value: "1234" },
    ]);
  });

  it("Drops keyless rows from the serialized value", () => {
    render(ProjectMemorySchemaFields, {
      open: true,
      rows: [
        { key: "dev", value: "npm run dev" },
        { key: "  ", value: "orphan" },
      ],
    });
    const hidden = document.querySelector('input[name="memory_schema_json"]') as HTMLInputElement;
    expect(JSON.parse(hidden.value)).toEqual([{ key: "dev", value: "npm run dev" }]);
  });

  it("Keeps the rows collapsed until the header is toggled open", async () => {
    let open = false;
    render(ProjectMemorySchemaFields, {
      open,
      rows: [{ key: "dev", value: "npm run dev" }],
      onOpenChange: (v: boolean) => (open = v),
    });
    expect(rowsOf().length).toBe(0);
    await fireEvent.click(screen.getByText(/Session memory/));
    await tick();
    expect(open).toBe(true);
  });

  it("Reports an added key through the onchange callback", async () => {
    let latest: { key: string; value: string }[] = [];
    render(ProjectMemorySchemaFields, {
      open: true,
      rows: [],
      onchange: (r: { key: string; value: string }[]) => (latest = r),
    });
    await fireEvent.click(screen.getByTestId("add-memory-key"));
    expect(latest).toEqual([{ key: "", value: "" }]);
  });

  it("Reports edited key and value text through the onchange callback", async () => {
    let latest: { key: string; value: string }[] = [];
    render(ProjectMemorySchemaFields, {
      open: true,
      rows: [{ key: "dev", value: "npm run dev" }],
      onchange: (r: { key: string; value: string }[]) => (latest = r),
    });
    await fireEvent.input(screen.getByLabelText("Memory default 1"), {
      target: { value: "pnpm dev" },
    });
    expect(latest).toEqual([{ key: "dev", value: "pnpm dev" }]);
  });

  it("Reports a removed row through the onchange callback", async () => {
    let latest: { key: string; value: string }[] = [];
    render(ProjectMemorySchemaFields, {
      open: true,
      rows: [
        { key: "dev", value: "npm run dev" },
        { key: "port", value: "1234" },
      ],
      onchange: (r: { key: string; value: string }[]) => (latest = r),
    });
    await fireEvent.click(screen.getByLabelText("Remove memory key 1"));
    expect(latest).toEqual([{ key: "port", value: "1234" }]);
  });
});

describe("EditAiForms", () => {
  it("Renders the three hidden action forms with correct actions", () => {
    render(EditAiForms, { draftDescription: "" });
    expect(document.querySelector("form#beautify-form")?.getAttribute("action")).toBe(
      "?/beautifyDescription",
    );
    expect(document.querySelector("form#generate-tasks-form")?.getAttribute("action")).toBe(
      "?/generateTasks",
    );
    expect(document.querySelector("form#import-tasks-form")?.getAttribute("action")).toBe(
      "?/importTasks",
    );
  });

  it("Binds the description to the hidden beautify/generate inputs", () => {
    render(EditAiForms, { draftDescription: "my description" });
    const descInputs = Array.from(
      document.querySelectorAll('input[name="description"]'),
    ) as HTMLInputElement[];
    expect(descInputs.length).toBe(2);
    for (const el of descInputs) expect(el.value).toBe("my description");
  });

  it("Renders the import tasks_json hidden field", () => {
    render(EditAiForms, { draftTasks: null });
    const input = document.querySelector('input[name="tasks_json"]') as HTMLInputElement;
    expect(input).not.toBeNull();
    expect(JSON.parse(input.value)).toEqual([]);
  });
});
