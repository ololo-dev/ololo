// @ts-nocheck — Storybook v10 types don't yet fully support Svelte 5 runes-mode components.
import type { Meta, StoryObj } from "@storybook/sveltekit";
import SettingsSidebar from "$lib/components/settings/SettingsSidebar.svelte";

const tabs = [
  { id: "general", label: "General", href: "/settings/general" },
  { id: "ai", label: "AI", href: "/settings/ai" },
  { id: "telemetry", label: "Telemetry", href: "/settings/telemetry" },
  { id: "projects", label: "Projects", href: "/settings/projects", count: 7 },
  { id: "users", label: "Users", href: "/settings/users", count: 24 },
  { id: "game_servers", label: "Game Servers", href: "/settings/game-servers" },
  { id: "email", label: "Email", href: "/settings/email" },
  { id: "categories", label: "Categories", href: "/settings/categories", count: 5 },
  { id: "judges", label: "Judges", href: "/settings/judges", count: 9 },
];

const meta = {
  title: "Settings/SettingsSidebar",
  component: SettingsSidebar,
  tags: ["autodocs"],
  parameters: {
    backgrounds: { default: "light-blue" },
  },
  argTypes: {
    collapsed: { control: "boolean" },
  },
} satisfies Meta<SettingsSidebar>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Expanded: Story = {
  args: { tabs, activeId: "judges", collapsed: false },
};

/** Collapsed to icons — labels live in the tooltip. */
export const Collapsed: Story = {
  args: { tabs, activeId: "judges", collapsed: true },
};

/** The first section, which is what a fresh admin lands on. */
export const FirstSectionActive: Story = {
  args: { tabs, activeId: "general", collapsed: false },
};
