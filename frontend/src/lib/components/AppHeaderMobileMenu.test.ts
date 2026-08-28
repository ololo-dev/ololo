import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/svelte";
import AppHeaderMobileMenu from "./AppHeaderMobileMenu.svelte";

const baseProps = {
  navItems: [{ label: "Projects", href: "/projects" }],
  isAuthenticated: false,
  isAdmin: false,
  showNewProject: false,
  user: { name: "" },
  isDocRoute: false,
  docSections: [],
  activeSlug: "",
};

describe("AppHeaderMobileMenu", () => {
  afterEach(() => cleanup());

  it("closes_the_menu_before_opening_the_auth_modal", async () => {
    // An overlay left open keeps its focus trap alive under the auth modal,
    // which on mobile yanked focus out of the email/password inputs.
    const onLogin = vi.fn();
    render(AppHeaderMobileMenu, {
      ...baseProps,
      menuOpen: true,
      onLogin,
    });
    const overlay = document.querySelector('[role="dialog"]') as HTMLElement;
    expect(overlay.classList.contains("opacity-100")).toBe(true);
    await fireEvent.click(screen.getByText("Log In"));
    expect(onLogin).toHaveBeenCalledOnce();
    // The overlay closed: its open-state classes are gone.
    expect(overlay.classList.contains("opacity-100")).toBe(false);
  });

  it("focus_trap_is_inert_while_the_menu_is_closed", async () => {
    render(AppHeaderMobileMenu, { ...baseProps, menuOpen: false });
    const overlay = document.querySelector('[role="dialog"]') as HTMLElement;
    const first = overlay.querySelector("a[href]") as HTMLElement;
    const focusSpy = vi.spyOn(first, "focus");

    // Focus leaves the (closed) overlay for somewhere outside — e.g. the
    // auth modal's email input. The trap must not steal it back.
    const outside = document.createElement("input");
    document.body.appendChild(outside);
    await fireEvent.focusOut(overlay, { relatedTarget: outside });

    expect(focusSpy).not.toHaveBeenCalled();
    outside.remove();
  });

  it("focus_trap_still_recaptures_while_the_menu_is_open", async () => {
    render(AppHeaderMobileMenu, { ...baseProps, menuOpen: true });
    const overlay = document.querySelector('[role="dialog"]') as HTMLElement;
    const first = overlay.querySelector("a[href], button:not([disabled])") as HTMLElement;
    const focusSpy = vi.spyOn(first, "focus");

    const outside = document.createElement("input");
    document.body.appendChild(outside);
    await fireEvent.focusOut(overlay, { relatedTarget: outside });

    expect(focusSpy).toHaveBeenCalled();
    outside.remove();
  });
});
