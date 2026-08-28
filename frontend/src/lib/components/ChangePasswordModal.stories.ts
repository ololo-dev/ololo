// @ts-nocheck — Storybook v10 types don't yet fully support Svelte 5 runes-mode components.
import type { Meta, StoryObj } from "@storybook/sveltekit";
import ChangePasswordModal from "$lib/components/ChangePasswordModal.svelte";

const meta = {
  title: "Components/ChangePasswordModal",
  component: ChangePasswordModal,
  tags: ["autodocs"],
  argTypes: {
    open: { control: "boolean" },
    error: { control: "text" },
    loading: { control: "boolean" },
  },
} satisfies Meta<typeof ChangePasswordModal>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Open: Story = {
  args: {
    open: true,
  },
};

export const WithError: Story = {
  args: {
    open: true,
    error: "Current password is incorrect.",
  },
};

export const Loading: Story = {
  args: {
    open: true,
    loading: true,
  },
};

export const Closed: Story = {
  args: {
    open: false,
  },
};
