// A personal project drafted outside /projects/new — the landing's
// "describe your work" popup — and handed to its form: kept in this tab's
// sessionStorage, read when the page is opened with `?draft=1`. Storage can
// be missing or refuse writes (a private window, blocked site data); the
// handoff then simply does not happen and the form starts empty.

const KEY = "ololo.projectDraft";

export interface ProjectDraft {
  name: string;
  description: string;
  tasks: { title: string; description: string }[];
  judges: string[];
  session_duration_secs: number;
  public: boolean;
  repo_url: string;
  repo_ref: string;
}

/** Keep `draft` for the next /projects/new?draft=1 in this tab. */
export function saveProjectDraft(draft: ProjectDraft): boolean {
  try {
    sessionStorage.setItem(KEY, JSON.stringify(draft));
    return true;
  } catch {
    return false;
  }
}

/** The draft kept for this tab, if any and well-formed. */
export function readProjectDraft(): ProjectDraft | null {
  let raw: string | null;
  try {
    raw = sessionStorage.getItem(KEY);
  } catch {
    return null;
  }
  if (!raw) return null;
  try {
    return normalize(JSON.parse(raw));
  } catch {
    return null;
  }
}

function text(v: unknown): string {
  return typeof v === "string" ? v : "";
}

/** Whatever the storage holds, as a draft — or null when it is not one. */
function normalize(v: unknown): ProjectDraft | null {
  if (!v || typeof v !== "object") return null;
  const d = v as Record<string, unknown>;
  const description = text(d.description).trim();
  if (!description) return null;
  const tasks = (Array.isArray(d.tasks) ? d.tasks : [])
    .map((t) => {
      const task = (t ?? {}) as Record<string, unknown>;
      return { title: text(task.title).trim(), description: text(task.description).trim() };
    })
    .filter((t) => t.title.length > 0);
  const secs = Number(d.session_duration_secs);
  return {
    name: text(d.name).trim(),
    description,
    tasks,
    judges: (Array.isArray(d.judges) ? d.judges : []).filter(
      (s): s is string => typeof s === "string",
    ),
    session_duration_secs: Number.isFinite(secs) && secs > 0 ? secs : 0,
    public: d.public !== false,
    repo_url: text(d.repo_url).trim(),
    repo_ref: text(d.repo_ref).trim(),
  };
}
