// A project's git repository, as the UI shows it.

/** A repository URL the way a person says it — `github.com/org/app` —
 *  whichever way it is spelled: https, ssh or `git@host:org/app`. */
export function repoLabel(url: string | null | undefined): string | null {
  const trimmed = (url ?? "").trim();
  if (!trimmed) return null;
  const rest = trimmed.includes("://")
    ? trimmed.split("://")[1]
    : trimmed.replace(/^[^@]*@/, "").replace(":", "/");
  return rest
    .replace(/^[^@/]*@/, "")
    .replace(/:\d+\//, "/")
    .replace(/\/+$/, "")
    .replace(/\.git$/, "");
}

/** The page to open for a repository: its https URL without `.git`. An ssh
 *  remote gets none — its web address is anyone's guess. */
export function repoWebUrl(url: string | null | undefined): string | null {
  const trimmed = (url ?? "").trim();
  if (!/^https:\/\/[^/@]+\/.+/.test(trimmed)) return null;
  return trimmed.replace(/\/+$/, "").replace(/\.git$/, "");
}
