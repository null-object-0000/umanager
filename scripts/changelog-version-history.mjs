// Paged JSON version-history changelog parsing for the CI feed generator.
//
// HexHub's update-log page (https://www.hexhub.cn/history) is a Nuxt SPA that
// renders its version list from
// `https://api.hexhub.cn/client/version/history-exact?size=100&offset=0`:
//
//   { "code": 0,
//     "data": { "content": [
//       { "id": "225", "clientType": "desktop", "version": "519",
//         "versionName": "5.1.9", "hotUpdate": false, "env": "production",
//         "created": 1780839719931,                // epoch *milliseconds*
//         "description": "[🔥] …</br>\n[✨] …" },  // HTML fragment
//       … newest first …
//     ] } }
//
// Two vendor quirks drive the selection rules below:
//   * `hotUpdate: true` marks patch-only releases that the desktop download
//     endpoint never serves — HexHub's `/latest-non-hot-detail` keeps handing
//     out the newest *full* installer (`hotUpdate: false`), which is exactly the
//     version the feed publishes. So the newest entry is usually the wrong one.
//   * `env` separates `beta` builds from `production` ones.
// An exact `versionName` match therefore wins (production preferred), then the
// newest production non-hot entry is used as a fallback.
//
// Only the first page is requested — the same `size=100&offset=0` the vendor's
// own page uses (`offset` is a page index, not a record offset, so page two
// would be `offset=1`). The newest 100 records reach back to 2.3.2 / 2025-02
// and the feed only ever publishes the newest full installer, so the target
// record is always on that page.
//
// `description` is an HTML fragment whose line breaks are written as the
// (technically invalid, browser-tolerant) `</br>` end tag, with `<h5>` section
// headings and `<b>` emphasis; it is normalized into a Markdown bullet list.
//
// All parsing happens in CI; the desktop app only ever reads the signed feed.

import { htmlToMarkdown } from "./changelog-atom.mjs";

/**
 * Extract the `data.content` array (newest first) from a version-history API
 * response.
 *
 * @param {unknown} text
 * @returns {Array<object>|null}
 */
export function parseVersionHistoryJson(text) {
  if (typeof text !== "string") return null;
  try {
    const payload = JSON.parse(text);
    const content = payload?.data?.content;
    return Array.isArray(content) ? content : null;
  } catch {
    return null;
  }
}

// The feed carries the Debian control version (`5.1.9`), the vendor's
// `versionName` is the same dotted string; normalize both sides so a Debian
// epoch (`1:5.1.9`), a `v` prefix or a revision suffix cannot defeat the match.
function normalizeVersion(value) {
  if (value == null) return "";
  const text = String(value).trim().replace(/^\d+:/, "");
  const match = text.match(/\d+(?:\.\d+)*/);
  return match ? match[0] : "";
}

function isProduction(entry) {
  return String(entry?.env ?? "").toLowerCase() !== "beta";
}

// The API returns real JSON booleans; the string forms are tolerated so a
// vendor-side serialization change cannot silently break the fallback.
function isHotUpdate(entry) {
  const value = entry?.hotUpdate;
  return value === true || value === "true";
}

/**
 * Select the history entry for `version`.
 *
 * An exact `versionName` match wins (a production record beats a beta one).
 * Without a match — a fresh installer whose history record has not been
 * published yet — the newest production non-hot entry is used, because that is
 * what the download endpoint is serving. Anything else yields null: picking the
 * newest entry outright would show a beta hot-update's notes for a stable
 * version.
 *
 * @param {unknown} entries
 * @param {string} [version]
 * @returns {object|null}
 */
export function selectVersionHistoryEntry(entries, version) {
  if (!Array.isArray(entries)) return null;
  const usable = entries.filter((entry) => entry && typeof entry === "object");
  if (usable.length === 0) return null;
  const target = normalizeVersion(version);
  if (target) {
    const matches = usable.filter((entry) => normalizeVersion(entry.versionName) === target);
    if (matches.length > 0) return matches.find(isProduction) ?? matches[0];
  }
  return (
    usable.find((entry) => isProduction(entry) && !isHotUpdate(entry)) ??
    usable.find(isProduction) ??
    null
  );
}

/**
 * True when the entry is the record for `version` itself — the only case in
 * which the entry's `created` may be reported as that version's official
 * publish time. A fallback selection belongs to a *different* version by
 * construction, so its date must never be published as `official` (that source
 * is always adopted by the merge, see `mergeVersionUpdatedAt`).
 *
 * @param {unknown} entry
 * @param {string} [version]
 * @returns {boolean}
 */
export function versionHistoryEntryMatchesVersion(entry, version) {
  const target = normalizeVersion(version);
  return Boolean(target) && normalizeVersion(entry?.versionName) === target;
}

// HexHub writes its changelog HTML with a handful of tags the shared converter
// does not know: `</br>` line breaks, `<h5>` section headings and `<b>`
// emphasis. Normalize them first, then reuse the shared HTML→Markdown rules.
// The vendor's `<h5>` sections (「数据库」「SSH」「通用」…) become `###` headings so
// they stay visually distinct from the bullet lines below them.
function normalizeDescriptionHtml(html) {
  return html
    .replace(/<\/br\s*\/?>/gi, "<br>")
    .replace(/<b(\s[^>]*)?>/gi, "<strong>")
    .replace(/<\/b\s*>/gi, "</strong>")
    .replace(/<h([4-6])\b[^>]*>([\s\S]*?)<\/h\1>/gi, "\n### $2\n");
}

// One release note per line: the vendor's own line breaks are the bullets, and
// every line must keep its own line in the rendered Markdown (a bare newline
// inside a paragraph would collapse into a space). Lines that are already a
// list item or a heading are left alone.
function bulletize(markdown) {
  return markdown
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => (/^([-*+]|\d+\.)\s|^#{1,6}\s/.test(line) ? line : `- ${line}`))
    .join("\n");
}

/**
 * Render one history entry's `description` as a Markdown bullet list.
 *
 * @param {unknown} entry
 * @returns {string}
 */
export function versionHistoryEntryToMarkdown(entry) {
  const description = typeof entry?.description === "string" ? entry.description : "";
  if (!description.trim()) return "";
  return bulletize(htmlToMarkdown(normalizeDescriptionHtml(description)));
}

/**
 * Parse an entry's `created` publish time (epoch milliseconds, seconds also
 * tolerated) into integer unix seconds, or null.
 *
 * @param {unknown} entry
 * @returns {number|null}
 */
export function versionHistoryEntryToUnixSeconds(entry) {
  const raw = entry?.created;
  const value = typeof raw === "string" ? Number(raw) : raw;
  if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) return null;
  // Epoch seconds stay ~1.8e9 for the next millennium; anything larger is
  // milliseconds.
  return Math.floor(value > 1e11 ? value / 1000 : value);
}
