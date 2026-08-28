/** Delivered captures, as a gallery reads them.
 *
 * A judge's artifact request lands in the repo as
 * `.ololo/artifacts/<request-id>/receipt-desktop.png`, and one request may
 * deliver several files. Three surfaces show those files — the report, the
 * judges tab and the chat — and each had grown its own copy of the same two
 * rules, which is how the report came to caption a picture with the whole
 * path and the position of a *different* file inside the delivery.
 */
import type { PlayerArtifactRef } from "$lib/types/arena";

export type GalleryEntry = {
  key: string;
  src: string;
  content_type: string;
  label: string;
};

/** `.ololo/artifacts/<request-id>/shot.png` → `shot.png`. */
export function fileName(path: string): string {
  return path.slice(path.lastIndexOf("/") + 1);
}

/**
 * One entry per delivered file of a request, in `?i=N` order.
 *
 * Each file is named by itself when the ref carries the list; a ref written
 * before that list existed falls back to the first file's name and the
 * position, which is wrong-ish but never blank.
 */
export function galleryEntries(
  a: PlayerArtifactRef,
  artifactUrl: (probeId: string, i: number) => string,
): GalleryEntry[] {
  const n = Math.max(1, a.file_count ?? 1);
  return Array.from({ length: n }, (_, i) => ({
    key: n > 1 ? `${a.probe_id}:${i}` : a.probe_id,
    src: artifactUrl(a.probe_id, i),
    content_type: a.content_type,
    label: a.files?.[i]
      ? fileName(a.files[i])
      : n > 1
        ? `${fileName(a.label)} (${i + 1}/${n})`
        : fileName(a.label),
  }));
}
