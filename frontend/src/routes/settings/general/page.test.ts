import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/svelte";
import GeneralSettings from "./+page.svelte";

vi.mock("$app/forms", () => ({ enhance: () => ({ destroy() {} }) }));
vi.mock("$lib/notifications.svelte", () => ({ notify: { success: vi.fn(), error: vi.fn() } }));

const data = {
  settings: {},
  allowUserProjectCreation: false,
  sessionReplayEnabled: true,
};

describe("General settings", () => {
  it("offers the replay switch, on by default", () => {
    render(GeneralSettings, { data: data as never });
    const toggle = screen.getByTestId("replay-setting-toggle") as HTMLInputElement;
    expect(toggle.checked).toBe(true);
    // The form posts what the switch shows, so a save cannot send the old value.
    const hidden = document.querySelector<HTMLInputElement>('input[name="replay_enabled"]');
    expect(hidden?.value).toBe("true");
  });

  it("shows the switch off when the instance turned it off", () => {
    render(GeneralSettings, { data: { ...data, sessionReplayEnabled: false } as never });
    const toggle = screen.getByTestId("replay-setting-toggle") as HTMLInputElement;
    expect(toggle.checked).toBe(false);
    const hidden = document.querySelector<HTMLInputElement>('input[name="replay_enabled"]');
    expect(hidden?.value).toBe("false");
  });

  it("says who the replay is for", () => {
    render(GeneralSettings, { data: data as never });
    expect(screen.getByText(/Administrators only/)).not.toBeNull();
  });
});
