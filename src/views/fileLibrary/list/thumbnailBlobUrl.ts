/**
 * Give a detached Grid thumbnail's in-flight blob: image request a chance to
 * settle before revoking its URL. The ownership is deliberately independent
 * of React's mounted element; cleanup can run before the image load event.
 *
 * A watchdog bounds retained Blob memory if decoding never finishes. It is
 * not used to suppress image or console errors from the browser QA gate.
 */
export const THUMBNAIL_BLOB_SETTLE_TIMEOUT_MS = 10_000;

export function releaseThumbnailBlobUrl(url: string, imageSettled: boolean): void {
  if (typeof URL === "undefined" || typeof URL.revokeObjectURL !== "function") return;
  if (imageSettled || typeof Image === "undefined") {
    URL.revokeObjectURL(url);
    return;
  }

  // Keep a detached image alive while its source is loading. An immediate
  // revoke here races the real Chromium <img> request after rapid unmounts.
  const probe = new Image();
  let released = false;
  let timeout: ReturnType<typeof setTimeout> | undefined;
  const release = () => {
    if (released) return;
    released = true;
    probe.onload = null;
    probe.onerror = null;
    if (timeout !== undefined) clearTimeout(timeout);
    URL.revokeObjectURL(url);
  };
  probe.onload = release;
  probe.onerror = release;
  timeout = setTimeout(release, THUMBNAIL_BLOB_SETTLE_TIMEOUT_MS);
  probe.src = url;
}
