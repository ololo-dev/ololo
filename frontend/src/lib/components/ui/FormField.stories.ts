// @ts-nocheck — Storybook v10 types don't yet fully support Svelte 5 runes-mode components.
import type { Meta, StoryObj } from "@storybook/sveltekit";
import FormField from "$lib/components/ui/FormField.svelte";

const meta = {
  title: "UI/FormField",
  component: FormField,
  tags: ["autodocs"],
  argTypes: {
    type: {
      control: "select",
      options: ["text", "email", "password", "textarea"],
    },
  },
} satisfies Meta<FormField>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  args: {
    label: "Your email",
    type: "email",
    placeholder: "Enter your email",
    name: "email",
  },
};

export const Password: Story = {
  args: {
    label: "Your password",
    type: "password",
    placeholder: "Enter your password",
    name: "password",
  },
};

export const WithError: Story = {
  args: {
    label: "Your email",
    type: "email",
    placeholder: "Enter your email",
    name: "email",
    value: "notanemail",
    error: "Please enter a valid email address",
  },
};

export const WithHint: Story = {
  args: {
    label: "Username",
    type: "text",
    placeholder: "e.g. johndoe",
    name: "username",
    hint: "Must be at least 3 characters",
  },
};

export const Textarea: Story = {
  args: {
    label: "Bio",
    type: "textarea",
    placeholder: "Tell us about yourself…",
    name: "bio",
  },
};

export const Required: Story = {
  args: {
    label: "Email",
    type: "email",
    placeholder: "Enter your email",
    name: "email",
    required: true,
  },
};
