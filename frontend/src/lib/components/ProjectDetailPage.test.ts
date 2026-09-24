import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import ProjectDetailPage from "./ProjectDetailPage.svelte";
import type { Project, ProjectPart, Session } from "$lib/api";

// The page opens a project WebSocket on mount; the observer is not what these
// cases are about.
vi.mock("$lib/ws-project.svelte", () => ({
  WsProjectClient: class {
    sessionCountdowns = {};
    connect() {}
    disconnect() {}
  },
}));

function project(over: Partial<Project> = {}): Project {
  return {
    id: "p1",
    name: "Handmade PostgreSQL",
    slug: "handmade-postgresql",
    description: "",
    public: true,
    archived_at: null,
    owner_user_id: "owner",
    tags: [],
    category: "Reinvent the Wheel",
    task_count: 0,
    session_duration_secs: 900,
    cover_image_url: null,
    ...over,
  } as Project;
}

function session(status: string): Session {
  return {
    id: `s-${status}`,
    name: status,
    status,
    owner_id: null,
    project_id: "p1",
    created_at: "2026-08-20T10:00:00Z",
    join_code: "AAAAAA",
    player_count: 1,
  } as Session;
}

const boards = { players: [], season_players: [], season_start: "", parts_total: 5 };

function renderPage(p: Project, sessions: Session[] = [], parts: ProjectPart[] = []) {
  return render(ProjectDetailPage, {
    project: p,
    ssrSessions: sessions,
    judges: [],
    topPlayers: boards,
    taskPreview: [],
    parts,
    message: null,
    currentUserId: null,
    isAdmin: false,
  });
}

function part(over: Partial<ProjectPart> & { id: string; name: string }): ProjectPart {
  return {
    slug: over.name.toLowerCase().replace(/\s+/g, "-"),
    description: "",
    public: true,
    archived_at: null,
    owner_user_id: "owner",
    tags: [],
    category: "Product Build",
    task_count: 1,
    session_duration_secs: 2400,
    cover_image_url: null,
    part_ordinal: 0,
    state: "available",
    ...over,
  } as ProjectPart;
}

describe("ProjectDetailPage", () => {
  it("Offers no session furniture on a campaign, which hosts none", () => {
    // `ololo start <campaign>` is refused by the server: the tab could only
    // ever count to zero, and "Active session: No" read as a dead project.
    renderPage(project({ part_count: 5 }));

    expect(screen.queryByRole("tab", { name: /Sessions/ })).toBeNull();
    expect(screen.getByRole("tab", { name: /Parts/ })).not.toBeNull();
    expect(screen.queryByText("Active session")).toBeNull();
    expect(screen.queryByText("Sessions")).toBeNull();
    // The stat slot the session count vacated now counts parts.
    expect(screen.getAllByText("Parts").length).toBeGreaterThan(0);
  });

  it("Offers the part a campaign is due to play, from the campaign's own page", async () => {
    // The campaign hosts no sessions, but it knows which part comes next —
    // sending the reader off to find it was a step that answered nothing.
    renderPage(
      project({ part_count: 3 }),
      [],
      [
        part({ id: "a", name: "The Ledger", part_ordinal: 0, state: "completed" }),
        part({ id: "b", name: "Receipts", part_ordinal: 1, state: "available" }),
        part({ id: "c", name: "Currencies", part_ordinal: 2, state: "locked" }),
      ],
    );

    const next = screen.getByTestId("campaign-start-next");
    expect(next.textContent).toContain("Start part 2");
    await fireEvent.click(next);
    // The popup starts the part, not the campaign it belongs to.
    expect(screen.getByRole("dialog", { name: "Start session" }).textContent).toContain("receipts");
  });

  it("Says a finished campaign is finished rather than offering nothing", () => {
    renderPage(
      project({ part_count: 2 }),
      [],
      [
        part({ id: "a", name: "The Ledger", part_ordinal: 0, state: "completed" }),
        part({ id: "b", name: "Receipts", part_ordinal: 1, state: "completed" }),
      ],
    );
    expect(screen.getByText(/Campaign complete/)).not.toBeNull();
    expect(screen.queryByTestId("campaign-start-next")).toBeNull();
  });

  it("Keeps the sessions tab on an ordinary project", () => {
    renderPage(project({ task_count: 3 }), [session("running")]);

    expect(screen.getByRole("tab", { name: /Sessions/ })).not.toBeNull();
    expect(screen.getByText("Active session")).not.toBeNull();
  });

  describe("a personal project", () => {
    const personal = project({
      kind: "personal",
      public: false,
      slug: "csv-export",
      owner_user_id: "me",
    });

    function renderPersonal(
      p: Project,
      sessions: Session[],
      viewer: { currentUserId: string; isAdmin: boolean },
    ) {
      return render(ProjectDetailPage, {
        project: p,
        ssrSessions: sessions,
        judges: [],
        topPlayers: boards,
        taskPreview: [],
        parts: [],
        message: null,
        ...viewer,
      });
    }

    it("puts the line to run in the owner's repository right on the card", () => {
      renderPersonal(personal, [], { currentUserId: "me", isAdmin: false });

      expect(screen.getByTestId("personal-start").textContent).toContain("ololo start csv-export");
      // Nothing to open first, and no visibility to choose: it is private.
      expect(screen.queryByRole("button", { name: "Start session" })).toBeNull();
      expect(screen.queryByText("Visibility")).toBeNull();
      const edit = screen.getByTestId("personal-edit");
      expect(edit.getAttribute("href")).toBe("/projects/p1/personal");
      expect(edit.textContent).toContain("Edit tasks and judges");
    });

    it("offers a new version once it has been played", () => {
      renderPersonal(personal, [session("finished")], { currentUserId: "me", isAdmin: false });
      const edit = screen.getByTestId("personal-edit");
      expect(edit.getAttribute("href")).toBe("/projects/new?from=p1");
      expect(edit.textContent).toContain("Duplicate and edit");
    });

    it("offers no start to anyone but its owner", () => {
      renderPersonal(personal, [], { currentUserId: "admin", isAdmin: true });
      expect(screen.queryByTestId("personal-start")).toBeNull();
    });

    it("tells its owner who sees it, and where to change that", () => {
      renderPersonal(personal, [], { currentUserId: "me", isAdmin: false });
      expect(screen.getByTestId("project-visibility-badge").textContent).toMatch(
        /Your project · private/,
      );
      const line = screen.getByTestId("personal-visibility");
      expect(line.textContent).toContain("only you see it");
      expect(line.querySelector("a")?.getAttribute("href")).toBe("/projects/p1/personal");
    });

    it("introduces a public one to a visitor by its owner", () => {
      renderPersonal({ ...personal, public: true, owner_username: "andrey" } as Project, [], {
        currentUserId: "visitor",
        isAdmin: false,
      });
      const badge = screen.getByTestId("project-visibility-badge");
      expect(badge.textContent).toMatch(/Personal project · by\s+@andrey/);
      expect(badge.querySelector("a")?.getAttribute("href")).toBe("/u/andrey");
      expect(screen.queryByTestId("personal-visibility")).toBeNull();
    });

    it("sends an archived one to a new version instead of the command", () => {
      renderPersonal({ ...personal, archived_at: "2026-09-23T10:00:00Z" } as Project, [], {
        currentUserId: "me",
        isAdmin: false,
      });
      const start = screen.getByTestId("personal-start");
      expect(start.textContent).not.toContain("ololo start");
      expect(start.textContent).toContain("Archived");
      expect(screen.queryByTestId("personal-visibility")).toBeNull();
      expect(screen.getByTestId("personal-edit").getAttribute("href")).toBe(
        "/projects/new?from=p1",
      );
    });
  });
});
