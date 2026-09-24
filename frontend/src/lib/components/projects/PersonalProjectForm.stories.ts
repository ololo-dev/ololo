// @ts-nocheck — Storybook v10 types don't yet fully support Svelte 5 runes-mode components.
import type { Meta, StoryObj } from "@storybook/sveltekit";
import PersonalProjectForm from "$lib/components/projects/PersonalProjectForm.svelte";

const judge = (slug, name, description, criteria, isDefault) => ({
  slug,
  name,
  description,
  criteria,
  avatar_url: null,
  default: isDefault,
});

const options = {
  creation_allowed: true,
  private_allowed: true,
  judges: [
    judge(
      "correctness",
      "Correctness",
      "Scores how faithfully a submission implements its task — scenario and contract coverage verified against the code.",
      ["product"],
      true,
    ),
    judge(
      "code-quality",
      "Code Quality",
      "Evaluates the craft of a submission — complexity, duplication, readability, and maintainability.",
      ["cleanliness", "maintainability"],
      true,
    ),
    judge(
      "test-quality",
      "Test Quality",
      "Evaluates the submission's tests — meaningfulness, level mix, and trustworthiness.",
      ["tests"],
      true,
    ),
    judge(
      "architecture",
      "Architecture",
      "Evaluates the structure of a submission against the dependency rule.",
      ["architecture"],
      false,
    ),
    judge(
      "ux-review",
      "UX Review",
      "Reviews screenshots of a web build — UI/UX quality, accessibility, and mobile readiness.",
      ["ux", "accessibility", "mobile"],
      false,
    ),
  ],
  limits: {
    max_tasks: 10,
    max_judges: 6,
    max_name_chars: 120,
    max_description_chars: 8000,
    max_task_title_chars: 200,
    max_task_description_chars: 4000,
  },
  session: { min_secs: 1800, max_secs: 28800, default_secs: 7200 },
  suggest_available: true,
  task_points: 100,
};

const meta = {
  title: "Projects/PersonalProjectForm",
  component: PersonalProjectForm,
  parameters: { backgrounds: { default: "light-blue" }, layout: "padded" },
} satisfies Meta<typeof PersonalProjectForm>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Empty: Story = {
  args: { options, submitLabel: "Create project", cancelHref: "/projects" },
};

export const WithMap: Story = {
  args: {
    options,
    submitLabel: "Create project",
    cancelHref: "/projects",
    initial: {
      name: "CSV export for the reports page",
      description:
        "Add a CSV export to the reports page. It must honour the table's filters and include the totals row.",
      tasks: [
        {
          title: "Serve the report as CSV",
          description: "GET /reports.csv honours every table filter.",
          judges: ["correctness", "test-quality"],
        },
        { title: "Add the export button", description: "" },
        { title: "Document the export", description: "" },
      ],
      judges: ["correctness", "code-quality", "test-quality"],
      session_duration_secs: 7200,
    },
  },
};

export const WithError: Story = {
  args: {
    ...WithMap.args,
    error: {
      error: "invalid_personal_project",
      field: "tasks",
      detail: "a navigation map holds at most 10 tasks",
    },
  },
};

export const WithoutPremium: Story = {
  args: {
    options: { ...options, private_allowed: false },
    submitLabel: "Create project",
    cancelHref: "/projects",
  },
};
