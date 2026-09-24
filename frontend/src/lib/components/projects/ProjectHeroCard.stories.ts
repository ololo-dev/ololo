// @ts-nocheck — Storybook v10 types don't yet fully support Svelte 5 runes-mode components.
import type { Meta, StoryObj } from "@storybook/sveltekit";
import ProjectHeroCard from "$lib/components/projects/ProjectHeroCard.svelte";

const personal = {
  id: "0b6f7c1e-4d7a-4a51-9d0e-6a7f1c2b3d4e",
  name: "CSV export for the reports page",
  slug: "csv-export-for-the-reports-page",
  kind: "personal",
  description:
    "Add a CSV export to the reports page. It must honour the table's filters and include the totals row.",
  public: true,
  archived_at: null,
  owner_user_id: "me",
  tags: [],
  category: null,
  task_count: 3,
  session_duration_secs: 7200,
  cover_image_url: null,
  judge_review_count: 10,
};

const judges = [
  { slug: "correctness", name: "Correctness", description: "Does what the task asked" },
  { slug: "code-quality", name: "Code Quality", description: "The craft of the change" },
  { slug: "test-quality", name: "Test Quality", description: "The tests of the change" },
];

const meta = {
  title: "Projects/ProjectHeroCard",
  component: ProjectHeroCard,
  parameters: { layout: "padded" },
} satisfies Meta<typeof ProjectHeroCard>;

export default meta;
type Story = StoryObj<typeof meta>;

const base = {
  judges,
  currentUserId: "me",
  isAdmin: false,
  sessionCount: 0,
  hasActiveSessions: false,
  onStart: () => {},
};

export const PersonalNew: Story = { args: { ...base, project: personal } };

export const PersonalPlayed: Story = {
  args: { ...base, project: personal, sessionCount: 2 },
};

export const PersonalArchived: Story = {
  args: { ...base, project: { ...personal, archived_at: "2026-09-23T10:00:00Z" } },
};

export const PersonalVisitor: Story = {
  args: {
    ...base,
    currentUserId: "visitor",
    project: { ...personal, public: true, owner_username: "andrey" },
    sessionCount: 3,
  },
};
