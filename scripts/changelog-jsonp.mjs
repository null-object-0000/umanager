// JSONP changelog parsing for the CI feed generator.
//
// QQ's Linux changelog is served as a JSONP script (the `#/log` SPA route loads
// `.../rainbow/linuxLog.js`), whose payload is `var params = [{version, date,
// feature:[...]}, ...]`. This module extracts that array, selects the entry for
// the resolved version, and renders its `feature` list as Markdown.
//
// All parsing happens in CI; the desktop app only ever reads the signed feed.

/**
 * Extract the `params` array from a JSONP changelog script.
 *
 * @param {unknown} text
 * @returns {Array<object>|null}
 */
export function parseJsonpChangelog(text) {
  if (typeof text !== "string") return null;
  const match = text.match(/var\s+params\s*=\s*(\[[\s\S]*?\])\s*;/);
  if (!match) return null;
  try {
    const entries = JSON.parse(match[1]);
    return Array.isArray(entries) ? entries : null;
  } catch {
    return null;
  }
}

/**
 * Select the changelog entry matching `version`.
 *
 * Entry versions look like `QQ Linux版 3.2.32` while the feed carries `3.2.32`
 * (websiteVersion) or `3.2.32-52194` (control version). Matching compares the
 * numeric part of the entry version against the numeric part of `version`, with
 * a substring fallback; if nothing matches, the first (latest) entry is used.
 *
 * @param {unknown} entries
 * @param {string} [version]
 * @returns {object|null}
 */
export function selectJsonpChangelogEntry(entries, version) {
  if (!Array.isArray(entries) || entries.length === 0) return null;
  if (version) {
    const target = String(version);
    const normalized = target.split(/[-~+]/)[0];
    const found = entries.find((entry) => {
      const label = typeof entry?.version === "string" ? entry.version : "";
      const numeric = (label.match(/\d+(?:\.\d+)+/) ?? [""])[0];
      return Boolean(numeric) && (numeric === normalized || label.includes(target));
    });
    if (found) return found;
  }
  return entries[0] ?? null;
}

/**
 * Render a changelog entry's `feature` list as Markdown.
 *
 * Section labels (e.g. `本次更新：`, `近期更新：`) become bold lines; `- …`
 * bullets stay a Markdown list.
 *
 * @param {unknown} entry
 * @returns {string}
 */
export function jsonpEntryToMarkdown(entry) {
  const feature = entry?.feature;
  if (!Array.isArray(feature)) return "";
  const lines = [];
  for (const raw of feature) {
    if (typeof raw !== "string") continue;
    const line = raw.trim();
    if (!line) continue;
    if (/^[-*]\s/.test(line)) {
      lines.push(line);
    } else {
      if (lines.length > 0) lines.push("");
      lines.push(`**${line}**`);
    }
  }
  return lines.join("\n").trim();
}
