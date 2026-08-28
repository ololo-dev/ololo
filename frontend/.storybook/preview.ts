import type { Preview } from "@storybook/sveltekit";
import "../src/app.css";

const preview: Preview = {
  parameters: {
    controls: {
      matchers: {
        color: /(background|color)$/i,
        date: /Date$/i,
      },
    },
    backgrounds: {
      default: "white",
      values: [
        { name: "white", value: "#ffffff" },
        { name: "light-blue", value: "#f4f8fe" },
      ],
    },
  },
};

export default preview;
