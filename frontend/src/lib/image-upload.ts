// Shared client-side gate for image uploads (avatars and project covers).
// Both upload straight to ImageKit, so this is the only place the accepted
// formats are decided — keeping them in one module stops the two uploaders
// from drifting apart.

/** Formats every current browser can render in an <img>. */
export const ACCEPTED_IMAGE_TYPES = [
  "image/jpeg",
  "image/png",
  "image/webp",
  "image/gif",
  "image/avif",
] as const;

/** Extensions matching ACCEPTED_IMAGE_TYPES, for the fallback below. */
const ACCEPTED_EXTENSIONS = ["jpg", "jpeg", "png", "webp", "gif", "avif"];

/**
 * `accept` attribute for the file input. Extensions are listed alongside the
 * MIME types because some OS/browser combinations report an empty type for
 * otherwise valid files, and would then grey them out in the picker.
 *
 * Deliberately excluded:
 * - SVG: can carry scripts, and we serve user uploads from a CDN origin.
 * - HEIC/HEIF: Safari renders it, Chrome and Firefox do not, so an uploaded
 *   HEIC would show as a broken image for most visitors.
 */
export const IMAGE_ACCEPT_ATTR = [
  ...ACCEPTED_IMAGE_TYPES,
  ...ACCEPTED_EXTENSIONS.map((e) => `.${e}`),
].join(",");

export const MAX_IMAGE_BYTES = 5 * 1024 * 1024;

/** Human-readable list for error copy, e.g. "JPG, PNG, WebP, GIF or AVIF". */
export const ACCEPTED_IMAGE_LABEL = "JPG, PNG, WebP, GIF or AVIF";

function extensionOf(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot === -1 ? "" : name.slice(dot + 1).toLowerCase();
}

/**
 * Validate a picked file. Returns an error message, or `undefined` when the
 * file is acceptable.
 *
 * Falls back to the filename extension when `file.type` is empty — browsers
 * do that for files whose MIME type the OS cannot resolve, and rejecting
 * those outright makes a perfectly valid JPEG un-uploadable.
 */
export function validateImageFile(file: File): string | undefined {
  const type = file.type.toLowerCase();
  const accepted: readonly string[] = ACCEPTED_IMAGE_TYPES;
  const typeOk = type
    ? accepted.includes(type)
    : ACCEPTED_EXTENSIONS.includes(extensionOf(file.name));

  if (!typeOk) {
    return `Unsupported image format. Use ${ACCEPTED_IMAGE_LABEL}.`;
  }
  if (file.size > MAX_IMAGE_BYTES) {
    return "File must be 5 MB or smaller.";
  }
  return undefined;
}
