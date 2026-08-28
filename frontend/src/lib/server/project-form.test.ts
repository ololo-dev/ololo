import { describe, it, expect } from "vitest";
import { buildMemorySchemaFromForm } from "./project-form";

function formWith(memorySchemaJson?: string) {
  const data = new FormData();
  if (memorySchemaJson !== undefined) data.set("memory_schema_json", memorySchemaJson);
  return data;
}

describe("buildMemorySchemaFromForm", () => {
  it("Omits the field entirely when the form carries no memory rows", () => {
    expect(buildMemorySchemaFromForm(formWith())).toBeUndefined();
  });

  it("Clears the schema when every row was removed", () => {
    expect(buildMemorySchemaFromForm(formWith("[]"))).toBeNull();
  });

  it("Builds a flat key -> default object", () => {
    const schema = buildMemorySchemaFromForm(
      formWith(JSON.stringify([{ key: "dev", value: "npm run dev" }])),
    );
    expect(schema).toEqual({ dev: "npm run dev" });
  });

  it("Sends numeric and boolean defaults as scalars, not strings", () => {
    const schema = buildMemorySchemaFromForm(
      formWith(
        JSON.stringify([
          { key: "port", value: "1234" },
          { key: "watch", value: "true" },
          { key: "off", value: "false" },
          { key: "cmd", value: "npm run dev" },
        ]),
      ),
    );
    expect(schema).toEqual({ port: 1234, watch: true, off: false, cmd: "npm run dev" });
  });

  it("Preserves whitespace inside string defaults", () => {
    const schema = buildMemorySchemaFromForm(
      formWith(JSON.stringify([{ key: "cmd", value: " npm run dev " }])),
    );
    expect(schema).toEqual({ cmd: " npm run dev " });
  });

  it("Drops rows with a blank key and clears when none survive", () => {
    expect(
      buildMemorySchemaFromForm(formWith(JSON.stringify([{ key: "  ", value: "x" }]))),
    ).toBeNull();
  });

  it("Leaves the schema untouched when the field is not parseable", () => {
    expect(buildMemorySchemaFromForm(formWith("not json"))).toBeUndefined();
    expect(buildMemorySchemaFromForm(formWith('{"dev":"x"}'))).toBeUndefined();
  });
});
