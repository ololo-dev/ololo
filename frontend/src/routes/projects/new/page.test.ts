import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/svelte";
import NewProjectPage from "./+page.svelte";

describe("routes/projects/new/+page.svelte", () => {
  const defaultData = {
    isAuthenticated: true,
    isAdmin: false,
    allowProjectCreation: false,
    replayEnabled: false,
    user: null,
    categories: [] as string[],
  };

  it("renders_name_input", () => {
    render(NewProjectPage, { data: defaultData, form: null });
    expect(screen.getByTestId("project-name")).not.toBeNull();
  });

  it("renders_description_textarea", () => {
    render(NewProjectPage, { data: defaultData, form: null });
    const el = document.querySelector('[data-testid="project-description"]');
    expect(el).not.toBeNull();
    expect(el?.tagName.toLowerCase()).toBe("textarea");
  });

  it("renders_public_checkbox_for_admin", () => {
    render(NewProjectPage, { data: { ...defaultData, isAdmin: true }, form: null });
    const el = screen.getByTestId("project-public") as HTMLInputElement;
    expect(el).not.toBeNull();
    expect(el.type).toBe("checkbox");
  });

  it("hides_public_checkbox_for_non_admin", () => {
    render(NewProjectPage, { data: defaultData, form: null });
    expect(screen.queryByTestId("project-public")).toBeNull();
  });

  it("renders_create_button", () => {
    render(NewProjectPage, { data: defaultData, form: null });
    expect(screen.getByTestId("create-project")).not.toBeNull();
  });

  it("renders_error_when_form_has_error", () => {
    render(NewProjectPage, {
      data: defaultData,
      form: {
        error: "invalid_name",
        name: "",
        description: "",
        public: "false",
      },
    });
    expect(screen.getByTestId("create-error")).not.toBeNull();
  });
});
