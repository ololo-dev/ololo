import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/svelte";
import AuthModal from "./AuthModal.svelte";

vi.mock("$app/forms", () => ({
  enhance: vi.fn(() => ({ destroy: vi.fn() })),
}));

vi.mock("$app/navigation", () => ({
  invalidate: vi.fn(),
}));

describe("AuthModal", () => {
  afterEach(() => cleanup());
  it("does_not_show_error_element_initially", () => {
    render(AuthModal, { open: true, mode: "login" });
    expect(screen.queryByTestId("login-modal-error")).toBeNull();
    expect(screen.queryByTestId("register-modal-error")).toBeNull();
  });

  it("shows_login_form_when_open_and_hides_when_closed", () => {
    const { unmount } = render(AuthModal, { open: true, mode: "login" });
    expect(document.querySelector("h2")?.textContent?.trim()).toBe("Log in");
    unmount();

    render(AuthModal, { open: false, mode: "login" });
    expect(document.querySelector("h2")).toBeNull();
  });

  it("switches_mode_between_login_and_register", async () => {
    render(AuthModal, { open: true, mode: "login" });

    expect(document.querySelector("h2")?.textContent?.trim()).toBe("Log in");
    expect(screen.queryByText("Create new account")).toBeNull();

    await fireEvent.click(screen.getByText("Sign Up"));
    expect(document.querySelector("h2")?.textContent?.trim()).toBe("Create new account");

    await fireEvent.click(screen.getByText("Log In"));
    expect(document.querySelector("h2")?.textContent?.trim()).toBe("Log in");
  });

  it("accepts_onclose_prop_without_error", () => {
    const onclose = vi.fn();
    expect(() => render(AuthModal, { open: true, onclose })).not.toThrow();
    expect(document.querySelector("h2")?.textContent?.trim()).toBe("Log in");
  });
});

describe("AuthModal email-only auth surface", () => {
  it("offers_no_social_sign_in_or_newsletter_box_on_either_form", () => {
    for (const mode of ["login", "register"] as const) {
      const { queryByTestId, unmount } = render(AuthModal, { open: true, mode });
      expect(document.querySelector('a[href^="/auth/oauth/"]')).toBeNull();
      expect(document.querySelector('input[name="subscribe_newsletter"]')).toBeNull();
      expect(queryByTestId("register-newsletter-optin")).toBeNull();
      unmount();
    }
  });

  it("still_offers_the_magic_link_path_from_the_login_form", async () => {
    render(AuthModal, { open: true, mode: "login" });
    await fireEvent.click(screen.getByText("Magic link"));
    expect(document.querySelector("h2")?.textContent?.trim()).toBe("Log in with magic link");
  });
});

describe("AuthModal sign-up password controls", () => {
  const passwordFields = () =>
    [
      document.querySelector("#auth-reg-password"),
      document.querySelector("#auth-reg-repeat-password"),
    ] as HTMLInputElement[];

  it("reveals_and_hides_both_password_fields_together", async () => {
    const { getByTestId } = render(AuthModal, { open: true, mode: "register" });
    const toggle = getByTestId("register-toggle-password");

    // Masked until asked, and the repeat field must not lag behind the first.
    expect(passwordFields().map((f) => f.type)).toEqual(["password", "password"]);
    expect(toggle.textContent?.trim()).toBe("Show password");

    await fireEvent.click(toggle);
    expect(passwordFields().map((f) => f.type)).toEqual(["text", "text"]);
    expect(toggle.textContent?.trim()).toBe("Hide password");
    expect(toggle.getAttribute("aria-pressed")).toBe("true");

    await fireEvent.click(toggle);
    expect(passwordFields().map((f) => f.type)).toEqual(["password", "password"]);
  });

  it("shows_no_strength_meter_until_something_is_typed", () => {
    const { queryByTestId } = render(AuthModal, { open: true, mode: "register" });
    expect(queryByTestId("password-strength")).toBeNull();
  });

  it("grades_the_password_as_it_is_typed", async () => {
    const { getByTestId } = render(AuthModal, { open: true, mode: "register" });
    const [password] = passwordFields();

    await fireEvent.input(password, { target: { value: "short" } });
    expect(getByTestId("password-strength-label").textContent?.trim()).toBe("Too short");

    await fireEvent.input(password, { target: { value: "correcthorsebatterystaple" } });
    expect(getByTestId("password-strength-label").textContent?.trim()).toBe("Strong");
  });

  it("keeps_the_meter_off_the_login_form", async () => {
    const { queryByTestId } = render(AuthModal, { open: true, mode: "login" });
    const login = document.querySelector("#auth-login-password") as HTMLInputElement;
    await fireEvent.input(login, { target: { value: "whatever" } });
    // Grading a password someone already owns is noise, not help.
    expect(queryByTestId("password-strength")).toBeNull();
  });
});
