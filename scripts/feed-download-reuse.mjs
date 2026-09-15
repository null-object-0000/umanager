// Pure helpers for "the vendor .deb did not change — reuse the previous feed
// entry instead of downloading it again".
//
// Why this exists: three source kinds (`versionEndpoint`, the .deb-backed
// `releaseApi`, `stableDownloadEndpoint`) can only learn an app's authoritative
// version / size / SHA-256 by downloading the whole .deb, so every CI run
// re-downloaded roughly 4.5 GB from vendor CDNs (Trae 436 MB, Docker Desktop
// 440 MB, WPS 545 MB, Feishu 338 MB, …) even when nothing had shipped since the
// previous run. That is tolerable at a 6-hour cadence and unacceptable at the
// sub-hourly cadence the feed targets (GitHub's scheduler delivers ~4-5 runs/day
// for this repo — see DESIGN-feed-update-cadence.md §5).
//
// Safety: the desktop app verifies every downloaded .deb against the SHA-256 in
// the signed feed (AGENTS invariant 5), so reusing a stale entry while a vendor
// silently replaced the artifact would surface as a failed download. Every rule
// below therefore needs vendor-provided evidence that the bytes are the same,
// and *any* missing signal means "download it":
//
//   1. `digestProvesUnchanged` — the Releases API itself published the asset's
//      SHA-256 and it equals the previous entry's: a bit-identical proof.
//   2. URL identity + an unchanged vendor-published version string.
//   3. URL identity + opt-in `immutableDownloadUrl` (the URL embeds the
//      version/build, e.g. …/releases/stable/2.3.83560/…).
//   4. URL identity + a `Last-Modified` equal to the one the previous feed
//      recorded for that same artifact (a CDN "not modified" assertion).
//   5. Rotating signed URL + unchanged vendor version (`dynamicDownloadUrl`).
//
// Rules 2–5 additionally require the probe's `Content-Length` to equal the size
// recorded in the previous feed, which catches in-place rebuilds that keep both
// the URL and the version. A source can also opt out entirely with
// `forceDownload: true`.
//
// Known remaining full download (2026-09): Tencent Docs. Its download URL is a
// fixed `…/api/package/get?version_id=latest` endpoint and its official page
// renders the version in JavaScript, so it offers no version string, no
// immutable URL and — because the changelog's publish date is the authoritative
// version time — no comparable `Last-Modified` either. It therefore re-downloads
// its ~300 MB .deb every run; giving it a `versionField`/`pageVersionMarker`
// (or teaching the generator to treat an unchanged changelog date as a version
// signal) would remove the last few GB/day.

const SHA256_RE = /^[0-9a-f]{64}$/i;

/** Whether `value` is a 64-character hex SHA-256 digest. */
export function isSha256(value) {
  return typeof value === "string" && SHA256_RE.test(value);
}

/**
 * A `releaseApi` asset whose API-published digest equals the previous entry's
 * digest is bit-identical, so the .deb control `Version` the generator read from
 * it last time is still the current one. This is a proof, not a heuristic.
 *
 * @param {string} digest        SHA-256 from the Releases API (no scheme prefix)
 * @param {object|null|undefined} previousEntry entry from the previous feed
 * @returns {boolean}
 */
export function digestProvesUnchanged(digest, previousEntry) {
  if (!isSha256(digest) || !previousEntry) return false;
  return isSha256(previousEntry.sha256) && digest.toLowerCase() === previousEntry.sha256.toLowerCase();
}

/**
 * Whether a previous entry carries everything a reuse needs: a digest, a size,
 * an authoritative version and the download URL it was built from.
 */
export function previousEntryIsReusable(previousEntry) {
  return Boolean(
    previousEntry
      && isSha256(previousEntry.sha256)
      && Number.isFinite(previousEntry.size)
      && previousEntry.size > 0
      && typeof previousEntry.version === "string"
      && previousEntry.version.length > 0
      && typeof previousEntry.downloadUrl === "string"
      && previousEntry.downloadUrl.length > 0,
  );
}

/** `Last-Modified`-style equality for the previous entry's recorded timestamp. */
function serverModifiedMatches(previousEntry, lastModified) {
  return previousEntry.versionUpdatedAtSource === "serverModified"
    && Number.isFinite(previousEntry.versionUpdatedAtUnixSeconds)
    && lastModified === previousEntry.versionUpdatedAtUnixSeconds;
}

/**
 * Decide whether this run may reuse the previous feed entry instead of
 * downloading the .deb again.
 *
 * @param {object} options
 * @param {object|null|undefined} options.previousEntry  previous published entry
 * @param {string} options.rawUrl         the download URL this run resolved
 *                                        (pre-signing: entries store the raw URL)
 * @param {string|null} [options.websiteVersion] vendor-published version string
 * @param {{contentLength?:number|null, lastModified?:number|null}|null} [options.probe]
 *                                        cheap header/size probe of the artifact
 * @param {boolean} [options.immutableDownloadUrl] source opt-in: the URL embeds
 *                                        the version/build and never changes in place
 * @param {boolean} [options.dynamicDownloadUrl]   source opt-in: the URL is a
 *                                        rotating signed link, so URL identity
 *                                        cannot be required
 * @param {boolean} [options.forceDownload]        source opt-out
 * @returns {{skip:boolean, reason:string}} `reason` is stable and loggable
 */
export function decideDownloadSkip({
  previousEntry,
  rawUrl,
  websiteVersion = null,
  probe = null,
  immutableDownloadUrl = false,
  dynamicDownloadUrl = false,
  forceDownload = false,
} = {}) {
  if (forceDownload) return { skip: false, reason: "forced" };
  if (!previousEntry) return { skip: false, reason: "no-previous" };
  if (!previousEntryIsReusable(previousEntry)) return { skip: false, reason: "previous-incomplete" };

  const sameUrl = typeof rawUrl === "string" && rawUrl === previousEntry.downloadUrl;
  if (!sameUrl && !dynamicDownloadUrl) return { skip: false, reason: "url-changed" };

  const contentLength = probe?.contentLength;
  if (!Number.isFinite(contentLength) || contentLength <= 0) {
    return { skip: false, reason: "no-probe" };
  }
  if (contentLength !== previousEntry.size) return { skip: false, reason: "size-changed" };

  const versionSame = websiteVersion != null
    && previousEntry.websiteVersion != null
    && String(websiteVersion) === String(previousEntry.websiteVersion);

  if (versionSame && (sameUrl || dynamicDownloadUrl)) {
    return { skip: true, reason: sameUrl ? "unchanged" : "unchanged-dynamic-url" };
  }
  if (sameUrl && immutableDownloadUrl) {
    return { skip: true, reason: "unchanged-immutable-url" };
  }
  if (sameUrl && Number.isFinite(probe?.lastModified) && serverModifiedMatches(previousEntry, probe.lastModified)) {
    return { skip: true, reason: "unchanged-server-modified" };
  }
  if (!sameUrl) return { skip: false, reason: "url-changed" };
  if (websiteVersion != null && previousEntry.websiteVersion != null) {
    return { skip: false, reason: "version-changed" };
  }
  return { skip: false, reason: "version-unknown" };
}

/**
 * Total byte length from a `Content-Range: bytes 0-0/12345` header, or null.
 * Used to size an artifact whose CDN does not answer HEAD (e.g. Tencent Docs).
 */
export function parseContentRangeTotal(headerValue) {
  if (typeof headerValue !== "string") return null;
  const match = headerValue.match(/\/\s*(\d+)\s*$/);
  if (!match) return null;
  const total = Number(match[1]);
  return Number.isFinite(total) && total > 0 ? total : null;
}

/**
 * Human-readable one-line summary of the download volume a run skipped, for the
 * CI log. `skipped` is `[{applicationId, size, reason}]`.
 */
export function summarizeSkipped(skipped) {
  if (!skipped || skipped.length === 0) return "未跳过任何整包下载";
  const bytes = skipped.reduce((total, item) => total + (item.size || 0), 0);
  const byReason = new Map();
  for (const item of skipped) {
    byReason.set(item.reason, (byReason.get(item.reason) || 0) + 1);
  }
  const reasons = [...byReason.entries()].map(([reason, count]) => `${reason}×${count}`).join("、");
  const mib = bytes / (1024 * 1024);
  const size = mib >= 1024 ? `${(mib / 1024).toFixed(2)} GB` : `${mib.toFixed(0)} MB`;
  return `跳过 ${skipped.length} 个未变更的整包下载，省下 ${size}（${reasons}）`;
}
