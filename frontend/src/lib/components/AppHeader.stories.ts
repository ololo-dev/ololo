// @ts-nocheck — Storybook v10 types don't yet fully support Svelte 5 runes-mode components.
import type { Meta, StoryObj } from "@storybook/sveltekit";
import AppHeader from "$lib/components/AppHeader.svelte";

const meta = {
  title: "Layout/AppHeader",
  component: AppHeader,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
  },
  argTypes: {
    isAuthenticated: { control: "boolean" },
    isAdmin: { control: "boolean" },
  },
} satisfies Meta<AppHeader>;

export default meta;
type Story = StoryObj<typeof meta>;

export const LoggedOut: Story = {
  args: {
    isAuthenticated: false,
  },
};

export const LoggedIn: Story = {
  args: {
    isAuthenticated: true,
    isAdmin: false,
    user: { initials: "AK", name: "Andrey Kucherenko" },
  },
};

export const Admin: Story = {
  args: {
    isAuthenticated: true,
    isAdmin: true,
    user: { initials: "AK", name: "Andrey Kucherenko" },
  },
};

export const WithAvatar: Story = {
  args: {
    isAuthenticated: true,
    isAdmin: true,
    user: {
      initials: "AK",
      name: "Andrey Kucherenko",
      avatarUrl: "https://i.pravatar.cc/56?u=andrey",
      username: "andrey",
    },
  },
};
