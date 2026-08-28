// ImageKit URL transformations (https://imagekit.io/docs/image-resize-and-crop).
//
// User uploads (avatars, project covers) are served from ik.imagekit.io at
// their original size — multi-megapixel files rendered into 32px circles.
// These helpers append a `tr` query parameter so the CDN delivers an image
// sized for the slot it renders in.
//
// Avatars can also come from external hosts, which do not understand
// ImageKit parameters — non-ImageKit URLs pass through untouched, so
// callers can wrap every avatar unconditionally.

const IMAGEKIT_HOST = "ik.imagekit.io";

// Every delivery also asks for `f-auto`: the CDN picks the lightest format
// the requesting client accepts (AVIF/WebP, falling back to the original
// encoding for clients that accept neither) — ~30% off large covers at no
// visual cost. Content negotiation is per Accept header, so social-card
// crawlers that only speak JPEG still get JPEG.
const AUTO_FORMAT = "f-auto";

function withTransform(url: string, tr: string): string {
  try {
    const parsed = new URL(url);
    if (parsed.hostname !== IMAGEKIT_HOST) return url;
    // Compose the query by hand: URLSearchParams would percent-encode the
    // commas in the tr value, and ImageKit's documented syntax uses them raw.
    parsed.searchParams.delete("tr");
    const others = parsed.searchParams.toString();
    return `${parsed.origin}${parsed.pathname}?${others ? `${others}&` : ""}tr=${tr}`;
  } catch {
    return url; // relative/invalid URLs: leave alone
  }
}

/**
 * Square avatar crop. `sizePx` is the CSS pixel size of the slot; the CDN is
 * asked for 2× that for retina displays. Default centre crop keeps the exact
 * dimensions (c-maintain_ratio).
 */
export function ikAvatar(url: string | null | undefined, sizePx: number): string {
  if (!url) return "";
  const px = sizePx * 2;
  return withTransform(url, `w-${px},h-${px},${AUTO_FORMAT}`);
}

/**
 * Cover/banner crop to an exact box (2× the CSS size should be passed by the
 * caller where retina matters). `fo-auto` keeps the interesting part of the
 * image in frame when the aspect ratio changes.
 */
export function ikCover(url: string | null | undefined, w: number, h: number): string {
  if (!url) return "";
  return withTransform(url, `w-${w},h-${h},fo-auto,${AUTO_FORMAT}`);
}

/**
 * Width-bound resize without cropping, for backgrounds using
 * `background-size: cover` where the box height varies with content.
 */
export function ikWidth(url: string | null | undefined, w: number): string {
  if (!url) return "";
  return withTransform(url, `w-${w},${AUTO_FORMAT}`);
}
