// HTML changelog parsing for the CI feed generator.
//
// Some vendors publish their update log only as a versioned HTML page (WeChat's
// Linux updates page is the canonical case: the server renders each version's
// changelog as a list of `<h4>` items under "该版本主要更新如下"). This module
// turns that page into the `releaseNotes` Markdown the generator emits.
//
// All parsing happens in CI; the desktop app only ever reads the signed feed.

import { htmlToMarkdown } from "./changelog-atom.mjs";

const HTML_ENTITIES = [
  ["&lt;", "<"],
  ["&gt;", ">"],
  ["&quot;", '"'],
  ["&apos;", "'"],
  ["&#39;", "'"],
  ["&#x27;", "'"],
  ["&nbsp;", " "],
  ["&amp;", "&"],
];

function decodeHtmlEntities(text) {
  let output = text;
  for (const [from, to] of HTML_ENTITIES) output = output.split(from).join(to);
  return output;
}

/**
 * Extract changelog items from a versioned "updates" HTML page.
 *
 * Items are the text content of `<h4>` elements (WeChat renders each bullet as
 * `<h4>- …</h4>`). Residual tags are stripped and entities decoded.
 *
 * @param {unknown} html
 * @returns {string[]}
 */
export function parseHtmlChangelog(html) {
  if (typeof html !== "string") return [];
  const items = [];
  const pattern = /<h4>\s*([\s\S]*?)\s*<\/h4>/g;
  let match;
  while ((match = pattern.exec(html)) !== null) {
    const text = decodeHtmlEntities(match[1].replace(/<[^>]*>/g, "")).trim();
    if (text) items.push(text);
  }
  return items;
}

/**
 * Join extracted changelog items into Markdown, normalizing a leading list
 * marker so the feed entry is a valid Markdown list.
 *
 * @param {string[]} items
 * @returns {string}
 */
export function htmlChangelogToMarkdown(items) {
  if (!Array.isArray(items) || items.length === 0) return "";
  return items
    .map((item) => (/^\s*[-*]\s/.test(item) ? item : `- ${item}`))
    .join("\n");
}

/**
 * Extract a single version's section from a VitePress-style release-notes page
 * (CodeBuddy: `<h3>4.11.2 (2026-08-20)</h3>` headings, each followed by
 * `<p><strong>…</strong></p>` / `<ul><li>…</li></ul>` bodies).
 *
 * Returns the section as Markdown. The section whose heading version matches
 * `version` is used when found; otherwise the first `<h3>` section (the latest)
 * is used as a fallback.
 *
 * @param {unknown} html
 * @param {string} [version]
 * @returns {string}
 */
export function extractHtmlVersionSection(html, version) {
  if (typeof html !== "string") return "";
  const headingPattern = /<h3\b[^>]*>([\s\S]*?)<\/h3>/g;
  const sections = [];
  let match;
  while ((match = headingPattern.exec(html)) !== null) {
    const headingText = match[1].replace(/<[^>]*>/g, "").replace(/\u200b/g, "").trim();
    const headingVersion = (headingText.match(/^\d+(?:\.\d+)+/) ?? [null])[0];
    // Only `<h3>` headings that are actually version headings (`X.Y.Z`) count;
    // VitePress sidebar/nav `<h3>`s are skipped.
    if (headingVersion) sections.push({ index: match.index, version: headingVersion });
  }
  if (sections.length === 0) return "";
  let target = sections.findIndex((section) => version && section.version === version);
  if (target < 0) target = 0;
  const start = sections[target].index;
  const end = sections[target + 1]?.index ?? html.length;
  const section = html.slice(start, end)
    // Drop VitePress permalink anchors and normalize heading tags so the shared
    // converter recognizes them (`<h3 id="…">` → `<h3>`).
    .replace(/<a class="header-anchor"[^>]*>[\s\S]*?<\/a>/g, "")
    .replace(/<h([1-6])\b[^>]*>/g, "<h$1>");
  return htmlToMarkdown(section);
}
