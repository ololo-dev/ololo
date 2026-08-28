// @ts-nocheck — Storybook v10 types don't yet fully support Svelte 5 runes-mode components.
import type { Meta, StoryObj } from "@storybook/sveltekit";
import Button from "$lib/components/ui/Button.svelte";

const meta = {
  title: "UI/Button",
  component: Button,
  tags: ["autodocs"],
  argTypes: {
    variant: {
      control: "select",
      options: ["primary", "outline", "ghost", "danger"],
    },
    size: {
      control: "select",
      options: ["sm", "md", "lg", "full"],
    },
    disabled: { control: "boolean" },
  },
  args: {
    variant: "outline",
    size: "md",
  },
} satisfies Meta<Button>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Primary: Story = {
  args: {
    variant: "primary",
  },
  render: (args) => ({
    Component: Button,
    props: args,
    slots: { default: "Sign Up" },
  }),
};

export const Outline: Story = {
  args: {
    variant: "outline",
  },
  render: (args) => ({
    Component: Button,
    props: args,
    slots: { default: "Log In" },
  }),
};

export const Danger: Story = {
  args: {
    variant: "danger",
  },
  render: (args) => ({
    Component: Button,
    props: args,
    slots: { default: "Cancel" },
  }),
};

export const Large: Story = {
  args: {
    variant: "primary",
    size: "lg",
  },
  render: (args) => ({
    Component: Button,
    props: args,
    slots: { default: "Sign Up Now" },
  }),
};

export const FullWidth: Story = {
  args: {
    variant: "primary",
    size: "full",
  },
  render: (args) => ({
    Component: Button,
    props: args,
    slots: { default: "Log in" },
  }),
};

export const AsLink: Story = {
  args: {
    variant: "primary",
    href: "/register",
  },
  render: (args) => ({
    Component: Button,
    props: args,
    slots: { default: "Register" },
  }),
};
