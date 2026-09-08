// JSON-list changelog parsing for the CI feed generator.
//
// Baidu Netdisk's version-update page (yun.baidu.com/disk/version) loads its
// changelog from `/disk/cmsdata?platform=linux&page=1&num=100`, a plain JSON
// list (newest first) whose items look like:
//
//   { "title": "百度网盘Linux电脑客户端V8.7.0",
//     "version": "百度网盘Linux电脑客户端V8.7.0",
//     "publish": "2026-08-05 20:27:00",
//     "detail": [{ "title": "更新内容：", "more": ["- …", …] }] }
//
// This module extracts the list, selects the entry for the resolved version,
// and renders its `detail[].more` bullets as Markdown.
//
// All parsing happens in CI; the desktop app only ever reads the signed feed.

/**
 * Extract the `list` array from a JSON-list changelog response.
 *
 * @param {unknown} text
 * @returns {Array<object>|null}
 */
export function parseJsonListChangelog(text) {
  if (typeof text !== "string") return null;
  try {
    const payload = JSON.parse(text);
    return Array.isArray(payload?.list) ? payload.list : null;
  } catch {
    return null;
  }
}

/**
 * Select the changelog entry matching `version`.
 *
 * Entry versions look like `百度网盘Linux电脑客户端V8.7.0` while the feed
 * carries `8.7.0` (control version). Matching compares the numeric part of the
 * entry version against the numeric part of `version`, with a substring
 * fallback; if nothing matches, the first (latest) entry is used.
 *
 * @param {unknown} list
 * @param {string} [version]
 * @returns {object|null}
 */
export function selectJsonListChangelogEntry(list, version) {
  if (!Array.isArray(list) || list.length === 0) return null;
  if (version) {
    const target = String(version);
    const normalized = target.split(/[-~+]/)[0];
    const found = list.find((entry) => {
      if (!entry || typeof entry !== "object") return false;
      const label = String(entry.version ?? entry.title ?? "");
      const numeric = (label.match(/\d+(?:\.\d+)+/) ?? [""])[0];
      return Boolean(numeric) && (numeric === normalized || label.includes(target));
    });
    if (found) return found;
  }
  return list[0] ?? null;
}

/**
 * Render a changelog entry's `detail` blocks as Markdown: each block's `title`
 * becomes a bold line (section label or marketing heading), each `more` item a
 * `- …` bullet.
 *
 * @param {unknown} entry
 * @returns {string}
 */
export function jsonListEntryToMarkdown(entry) {
  const detail = entry?.detail;
  if (!Array.isArray(detail)) return "";
  const lines = [];
  for (const block of detail) {
    if (!block || typeof block !== "object") continue;
    const title = typeof block.title === "string" ? block.title.trim() : "";
    if (title) {
      if (/^[-*]\s/.test(title)) {
        lines.push(title);
      } else {
        if (lines.length > 0) lines.push("");
        lines.push(`**${title}**`);
      }
    }
    const more = Array.isArray(block.more) ? block.more : [];
    for (const raw of more) {
      if (typeof raw !== "string") continue;
      const line = raw.trim();
      if (!line) continue;
      lines.push(/^[-*]\s/.test(line) ? line : `- ${line}`);
    }
  }
  return lines.join("\n").trim();
}
