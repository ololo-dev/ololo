import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/svelte";
import SessionLobby from "./SessionLobby.svelte";

describe("SessionLobby", () => {
  it("renders agent_display_name next to participant name", () => {
    render(SessionLobby, {
      participants: [
        {
          user_id: "u1",
          display_name: "Alpha",
          joined_at: "2026-05-22T00:00:00Z",
          fingerprint: "AABBCCDDEEFF11223344",
          avatar_url: null,
          agent_display_name: "opencode",
        },
      ],
    });

    expect(screen.getByText("Alpha")).not.toBeNull();
    expect(screen.getByText(/opencode/)).not.toBeNull();
  });

  it("renders avatar image when avatar_url is present", () => {
    render(SessionLobby, {
      participants: [
        {
          user_id: "u1",
          display_name: "Alpha",
          joined_at: "2026-05-22T00:00:00Z",
          fingerprint: "0123456789abcdef",
          avatar_url: "https://example.com/avatar.png",
        },
      ],
    });

    const avatar = screen.getByRole("img", { name: "Alpha avatar" });
    expect(avatar).toHaveAttribute("src", "https://example.com/avatar.png");
  });

  it("renders empty state when no participants", () => {
    render(SessionLobby, { participants: [] });
    expect(screen.getByText("No players connected yet.")).not.toBeNull();
  });
});
