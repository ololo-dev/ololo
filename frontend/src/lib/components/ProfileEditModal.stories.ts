// @ts-nocheck — Storybook v10 types don't yet fully support Svelte 5 runes-mode components.
import type { Meta, StoryObj } from "@storybook/sveltekit";
import ProfileEditModal from "$lib/components/ProfileEditModal.svelte";

const meta = {
  title: "Components/ProfileEditModal",
  component: ProfileEditModal,
  tags: ["autodocs"],
  argTypes: {
    open: { control: "boolean" },
    displayName: { control: "text" },
    email: { control: "text" },
    error: { control: "text" },
    loading: { control: "boolean" },
  },
} satisfies Meta<typeof ProfileEditModal>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Open: Story = {
  args: {
    open: true,
    displayName: "Andrey Kucherenko",
    email: "andrey@example.com",
  },
};

export const WithError: Story = {
  args: {
    open: true,
    displayName: "Andrey Kucherenko",
    email: "andrey@example.com",
    error: "Please enter a valid display name.",
  },
};

export const Loading: Story = {
  args: {
    open: true,
    displayName: "Andrey Kucherenko",
    email: "andrey@example.com",
    loading: true,
  },
};

export const Closed: Story = {
  args: {
    open: false,
    displayName: "Andrey Kucherenko",
    email: "andrey@example.com",
  },
};
