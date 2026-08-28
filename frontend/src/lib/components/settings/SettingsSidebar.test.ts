import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import SettingsSidebar from "./SettingsSidebar.svelte";

const tabs = [
  { id: "general", label: "General", href: "/settings/general" },
  { id: "users", label: "Users", href: "/settings/users", count: 24 },
  { id: "judges", label: "Judges", href: "/settings/judges" },
];

describe("SettingsSidebar", () => {
  it("Lists every section with its link", () => {
    render(SettingsSidebar, { tabs, activeId: "general" });
    expect(screen.getByTestId("settings-tab-general").getAttribute("href")).toBe(
      "/settings/general",
    );
    expect(screen.getByTestId("settings-tab-users").getAttribute("href")).toBe("/settings/users");
    expect(screen.getByTestId("settings-tab-judges")).not.toBeNull();
  });

  it("Marks the open section for assistive tech, not only by colour", () => {
    render(SettingsSidebar, { tabs, activeId: "users" });
    expect(screen.getByTestId("settings-tab-users").getAttribute("aria-current")).toBe("page");
    expect(screen.getByTestId("settings-tab-general").getAttribute("aria-current")).toBeNull();
  });

  it("Shows a count only where there is something to count", () => {
    render(SettingsSidebar, { tabs, activeId: "general" });
    expect(screen.getByTestId("settings-tab-users").textContent).toContain("24");
    expect(screen.getByTestId("settings-tab-judges").textContent?.trim()).toBe("Judges");
  });

  it("Reports a collapse request instead of collapsing itself", async () => {
    const onToggle = vi.fn();
    render(SettingsSidebar, { tabs, activeId: "general", collapsed: false, onToggle });
    await fireEvent.click(screen.getByTestId("sidebar-toggle"));
    // The parent owns the state — it is the one that persists it.
    expect(onToggle).toHaveBeenCalledWith(true);
  });

  it("Asks to expand again when it is already collapsed", async () => {
    const onToggle = vi.fn();
    render(SettingsSidebar, { tabs, activeId: "general", collapsed: true, onToggle });
    const toggle = screen.getByTestId("sidebar-toggle");
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    await fireEvent.click(toggle);
    expect(onToggle).toHaveBeenCalledWith(false);
  });

  it("Keeps the label reachable as a tooltip once collapsed", () => {
    render(SettingsSidebar, { tabs, activeId: "general", collapsed: true });
    // The text is still in the DOM (hidden by a breakpoint class), and the
    // title carries it for a pointer user.
    expect(screen.getByTestId("settings-tab-users").getAttribute("title")).toBe("Users");
    expect(screen.getByTestId("settings-sidebar").getAttribute("data-collapsed")).toBe("true");
  });

  it("Drops the tooltip when expanded, where the label is visible", () => {
    render(SettingsSidebar, { tabs, activeId: "general", collapsed: false });
    expect(screen.getByTestId("settings-tab-users").getAttribute("title")).toBeNull();
  });
});

describe("SettingsSidebar on a phone", () => {
  it("Names the open section on the disclosure button", () => {
    render(SettingsSidebar, { tabs, activeId: "users" });
    const toggle = screen.getByTestId("sidebar-mobile-toggle");
    expect(toggle.textContent).toContain("Users");
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
  });

  it("Reveals every section on tap, so none hides off-screen", async () => {
    render(SettingsSidebar, { tabs, activeId: "users" });
    const toggle = screen.getByTestId("sidebar-mobile-toggle");
    const list = screen.getByTestId("settings-tab-list");
    // Collapsed on a phone; the md: class keeps it visible on a desktop.
    expect(list.className).toContain("hidden");

    await fireEvent.click(toggle);
    expect(list.className).toContain("flex");
    expect(list.className).not.toContain("hidden");
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
  });

  it("Falls back to the first section when the active id is unknown", () => {
    render(SettingsSidebar, { tabs, activeId: "nonexistent" });
    expect(screen.getByTestId("sidebar-mobile-toggle").textContent).toContain("General");
  });
});
