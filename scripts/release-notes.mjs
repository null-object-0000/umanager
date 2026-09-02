// Release-notes sanitization for the CI feed generator.
//
// GitHub release bodies are the canonical "version update record" for
// `releaseApi` sources. They ship as signed feed data (never fetched by the
// desktop app), so they must stay bounded and structurally sound. The desktop
// validator (`src-tauri/src/feed.rs` `validate_release_notes`) rejects a note
// longer than 200_000 UTF-8 bytes, so this module truncates at that same
// ceiling — the old 20_000-char cut was 10× tighter than the validator and
// dropped real changelog content. Extracted here so the byte-exact truncation
// can be unit-tested independently (see release-notes.test.mjs).

export const MAX_RELEASE_NOTES_BYTES = 200_000;

const TRUNCATION_SUFFIX = "\n\n…（内容过长，已截断，完整内容见发布页）";

// Fixed, vendor-specific boilerplate pinned to every release body. FlClash
// appends a cross-OS download matrix and a "full changelog" link that are not
// changelog content (and duplicate what the app already surfaces elsewhere), so
// strip them at generation time.
const RELEASE_NOTES_BOILERPLATE = {
  flclash: /\*{0,2}Download based on your OS:\*{0,2}[\s\S]*$/i,
};

/**
 * Remove a vendor's fixed, non-changelog footer from a release body.
 *
 * @param {unknown} body raw GitHub release body
 * @param {string} applicationId
 * @returns {unknown} the body with the boilerplate removed (or unchanged)
 */
export function stripReleaseNotesBoilerplate(body, applicationId) {
  if (typeof body !== "string") return body;
  const pattern = RELEASE_NOTES_BOILERPLATE[applicationId];
  return pattern ? body.replace(pattern, "") : body;
}

/**
 * Sanitize a GitHub release body into feed-safe release notes.
 *
 * - Non-strings become `null` (absent from the feed).
 * - NUL bytes are dropped and surrounding whitespace trimmed.
 * - Empty results become `null`.
 * - Very long bodies are truncated to `MAX_RELEASE_NOTES_BYTES` **bytes**
 *   (counting the suffix), never splitting a multi-byte UTF-8 sequence, so the
 *   final note always passes the desktop validator's `200_000` byte check.
 *
 * @param {unknown} body raw GitHub release body
 * @returns {string|null}
 */
export function sanitizeReleaseNotes(body) {
  if (typeof body !== "string") return null;
  const cleaned = body.replace(/\0/g, "").trim();
  if (!cleaned) return null;
  if (Buffer.byteLength(cleaned, "utf8") <= MAX_RELEASE_NOTES_BYTES) {
    return cleaned;
  }
  const suffixBytes = Buffer.byteLength(TRUNCATION_SUFFIX, "utf8");
  const buffer = Buffer.from(cleaned, "utf8");
  let end = MAX_RELEASE_NOTES_BYTES - suffixBytes;
  // Never split a UTF-8 continuation byte (0b10xxxxxx).
  while (end > 0 && (buffer[end] & 0xc0) === 0x80) end -= 1;
  return `${buffer.subarray(0, end).toString("utf8")}${TRUNCATION_SUFFIX}`;
}

/**
 * Pick the release whose changelog should be shown, from a GitHub Releases API
 * response — either a single release object or an array of releases.
 *
 * Drafts and prereleases are skipped. When `tagPrefix` is set (e.g. Bitwarden's
 * `desktop-` monorepo tags), only releases whose tag starts with it are
 * considered. Arrays are assumed newest-first (GitHub's default ordering), so
 * the first match wins.
 *
 * @param {unknown} payload GitHub release object or array
 * @param {string} [tagPrefix]
 * @returns {object|null}
 */
export function selectReleaseNotesRelease(payload, tagPrefix) {
  const releases = Array.isArray(payload) ? payload : [payload];
  const prefix = tagPrefix || "";
  return (
    releases.find((release) => {
      if (!release || typeof release !== "object") return false;
      if (release.draft || release.prerelease) return false;
      const tag = String(release.tag_name || "");
      return prefix ? tag.startsWith(prefix) : true;
    }) ?? null
  );
}
