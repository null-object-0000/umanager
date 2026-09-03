// Markdown changelog cleanup for the CI feed generator.
//
// Some vendors publish release notes as raw Markdown in a docs repository (VS
// Code's `microsoft/vscode-docs` is the canonical case). Those files carry
// YAML front matter and site-template directives / TOC / placeholder HTML
// comments that are not changelog content. This module strips that scaffolding
// so the feed ships only the release-notes Markdown.
//
// All parsing happens in CI; the desktop app only ever reads the signed feed.

/**
 * Remove a leading YAML front-matter block (`--- ... ---`) if present.
 *
 * @param {unknown} markdown
 * @returns {string}
 */
export function stripMarkdownFrontMatter(markdown) {
  if (typeof markdown !== "string") return "";
  const trimmed = markdown.replace(/^\uFEFF/, "");
  if (!trimmed.startsWith("---")) return trimmed;
  const end = trimmed.indexOf("\n---", 3);
  if (end < 0) return trimmed;
  return trimmed.slice(end + 4).replace(/^\s*\n/, "");
}

/**
 * Remove `<!-- ... -->` HTML comments (site-template directives, TOC blocks,
 * placeholders) from Markdown.
 *
 * @param {unknown} markdown
 * @returns {string}
 */
export function stripHtmlComments(markdown) {
  if (typeof markdown !== "string") return "";
  return markdown.replace(/<!--[\s\S]*?-->/g, "");
}

/**
 * Remove leftover raw HTML tags (e.g. a trailing `<link>` / scroll-to-top
 * anchor) that are site scaffolding rather than changelog text.
 *
 * @param {unknown} markdown
 * @returns {string}
 */
export function stripRawHtmlTags(markdown) {
  if (typeof markdown !== "string") return "";
  return markdown.replace(/<\/?[a-z][^>]*>/gi, "");
}

/**
 * Full cleanup pipeline for a docs-repository release-notes Markdown file.
 *
 * @param {unknown} markdown
 * @returns {string}
 */
export function cleanReleaseNotesMarkdown(markdown) {
  if (typeof markdown !== "string") return "";
  return stripRawHtmlTags(stripHtmlComments(stripMarkdownFrontMatter(markdown)));
}

/**
 * Extract a single version's section from a CHANGELOG.md-style Markdown file.
 *
 * The file is split into `## <version>` sections. Headings may be bare
 * (`## 2.1.258`, claude-code) or bracketed with a date suffix
 * (`## [0.84.4] - 2026-08-28`, Pi). The section whose heading carries `version`
 * is returned; otherwise the first `##` section (the latest) is the fallback.
 *
 * @param {unknown} markdown
 * @param {string} [version]
 * @returns {string}
 */
export function extractMarkdownVersionSection(markdown, version) {
  if (typeof markdown !== "string") return "";
  const lines = markdown.split(/\r?\n/);
  const headingVersion = (line) => {
    const match = line.trim().match(/^##\s+(.+?)\s*$/);
    if (!match) return null;
    const text = match[1];
    const numeric = text.match(/\d+(?:\.\d+)+(?:[-+][\w.-]+)?/);
    return numeric ? numeric[0] : text;
  };
  let target = -1;
  if (version) {
    const expected = String(version).replace(/^v/, "");
    target = lines.findIndex((line) => headingVersion(line) === expected);
  }
  if (target < 0) {
    target = lines.findIndex((line) => /^##\s+\S/.test(line));
  }
  if (target < 0) return "";
  const section = [];
  for (let index = target; index < lines.length; index += 1) {
    if (index > target && /^##\s/.test(lines[index])) break;
    section.push(lines[index]);
  }
  return section.join("\n").trim();
}

