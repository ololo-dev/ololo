import type { PlayerHistoryCommit, PlayerHistoryFileDiff } from "$lib/types/arena";

// ponytail: client-side reduction, intervening mod/delete collapse incorrectly;
// acceptable per Don. Upgrade: server-side range diff endpoint if fidelity needed.

const STATUS_PRECEDENCE: Record<string, number> = { added: 1, modified: 2, deleted: 3 };

export function reduceDiff(commits: PlayerHistoryCommit[]): PlayerHistoryFileDiff[] {
  if (commits.length === 0) return [];

  const ordered = [...commits].sort((a, b) => a.author_time.localeCompare(b.author_time));

  const byPath = new Map<string, { patches: string[]; lastStatus: string }>();

  for (const commit of ordered) {
    for (const file of commit.files) {
      const existing = byPath.get(file.path);
      if (!existing) {
        byPath.set(file.path, { patches: [file.patch], lastStatus: file.status });
        continue;
      }
      const prevPrec = STATUS_PRECEDENCE[existing.lastStatus] ?? 0;
      const nextPrec = STATUS_PRECEDENCE[file.status] ?? 0;
      if (nextPrec > prevPrec && file.status === "deleted") {
        existing.patches = [file.patch];
        existing.lastStatus = "deleted";
      } else {
        existing.patches.push(file.patch);
        existing.lastStatus = file.status;
      }
    }
  }

  const out: PlayerHistoryFileDiff[] = [];
  for (const [path, { patches, lastStatus }] of byPath) {
    out.push({ path, status: lastStatus, patch: patches.join("\n") });
  }
  out.sort((a, b) => a.path.localeCompare(b.path));
  return out;
}
