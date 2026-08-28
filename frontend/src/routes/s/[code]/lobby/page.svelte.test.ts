import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/svelte";
import LobbyPage from "./+page.svelte";

vi.mock("$app/navigation", () => ({
  goto: vi.fn(),
  invalidateAll: vi.fn(async () => {}),
}));

vi.mock("$app/environment", () => ({ browser: true }));

vi.mock("$lib/ws-session.svelte", () => ({
  WsSessionClient: class {
    countdownSecs = 0;
    runningCountdownSecs = null;
    leaderboard = [];
    participants = [];
    scoreHistory = [];
    userPlayers = [];
    phase = "lobby";
    protocolMismatch = false;
    degraded = null;
    connect() {}
    disconnect() {}
    constructor(_code: string, _token: string) {}
  },
}));

vi.mock("$lib/auth", () => ({
  getToken: () => null,
  initAuth: vi.fn(async () => {}),
}));

const baseSession = {
  id: "sess-1",
  join_code: "ABC123",
  state: "lobby",
  owner_id: null as string | null,
  project_id: "proj-1",
  project_name: "Project One",
  project_slug: "project-one",
  project_description: null,
};

const baseData = {
  session: baseSession,
  isAuthenticated: false,
  isAdmin: false,
  allowProjectCreation: false,
  replayEnabled: false,
  user: null as {
    id: string;
    name: string;
    initials: string;
    avatarUrl?: string;
    username?: string;
  } | null,
};

describe("routes/s/[code]/lobby/+page.svelte", () => {
  it("shows Cancel control to session owner in lobby", () => {
    render(LobbyPage, {
      data: {
        ...baseData,
        session: { ...baseSession, owner_id: "user-1" },
        isAuthenticated: true,
        isAdmin: false,
        user: {
          id: "user-1",
          name: "Owner",
          initials: "OW",
          avatarUrl: undefined,
          username: "owner",
        },
      },
    });
    expect(screen.getByRole("button", { name: /cancel/i })).not.toBeNull();
    // No pause in lobby
    expect(screen.queryByRole("button", { name: /pause|resume/i })).toBeNull();
  });

  it("shows Cancel control to admin in lobby", () => {
    render(LobbyPage, {
      data: {
        ...baseData,
        session: { ...baseSession, owner_id: "user-other" },
        isAuthenticated: true,
        isAdmin: true,
        user: {
          id: "user-1",
          name: "Admin",
          initials: "AD",
          avatarUrl: undefined,
          username: "admin",
        },
      },
    });
    expect(screen.getByRole("button", { name: /cancel/i })).not.toBeNull();
  });

  it("hides Cancel control from non-owner non-admin viewer in lobby", () => {
    render(LobbyPage, {
      data: {
        ...baseData,
        session: { ...baseSession, owner_id: "user-other" },
        isAuthenticated: true,
        isAdmin: false,
        user: {
          id: "user-1",
          name: "Spectator",
          initials: "SP",
          avatarUrl: undefined,
          username: "spec",
        },
      },
    });
    expect(screen.queryByRole("button", { name: /cancel|pause|resume/i })).toBeNull();
  });
});
