import type { Config } from "tailwindcss";
import typography from "@tailwindcss/typography";

const config: Config = {
  content: ["./src/**/*.{html,js,svelte,ts}"],
  theme: {
    extend: {
      colors: {
        brand: {
          blue: "#0269fb",
          red: "#fb341c",
          "light-blue": "#f4f8fe",
          text: "#363636",
          border: "#e7eaee",
          muted: "#9ea7b6",
          "check-blue": "#8fb4ec",
          "tag-bg": "#dce9fc",
          // The documentation's two remaining colours, named so pages other
          // than /documentation can speak the same language: the deep blue it
          // sets callout and table text in, and the tint behind table heads.
          "deep-blue": "#36547f",
          "table-head": "#eef4fd",
        },
      },
      fontFamily: {
        heading: ["Raleway", "sans-serif"],
        body: ["Raleway", "sans-serif"],
      },
      borderRadius: {
        btn: "8px",
      },
    },
  },
  plugins: [typography],
};

export default config;
