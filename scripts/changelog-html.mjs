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

/**
 * Extract the changelog bullet list for one platform block from a download
 * page (QQ Music). The block is located by `blockMarker` (a CSS class such as
 * `ic-linux`), and its changelog is the first `<ul class="version_list">` after
 * that marker. Items are the text of `<li class="version_list__item">` entries
 * (already `- …` bullets in the page).
 *
 * @param {unknown} html
 * @param {string} [blockMarker]
 * @returns {string[]}
 */
export function parseHtmlVersionList(html, blockMarker) {
  if (typeof html !== "string" || typeof blockMarker !== "string" || !blockMarker) return [];
  // Prefer a `<li … class="… blockMarker …">` occurrence: a marker like
  // `ic-linux` also appears inside `<style>` selectors, so a bare `indexOf`
  // would anchor the search to the wrong (earlier) block. A plain `indexOf`
  // is kept as a fallback for markers that aren't `<li>` class names.
  const escaped = blockMarker.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const liMatch = new RegExp(`<li\\b[^>]*\\b${escaped}\\b[^>]*>`).exec(html);
  let markerIndex = liMatch ? liMatch.index : html.indexOf(blockMarker);
  if (markerIndex < 0) return [];
  const listStart = html.indexOf('<ul class="version_list"', markerIndex);
  if (listStart < 0) return [];
  const listEnd = html.indexOf("</ul>", listStart);
  if (listEnd < 0) return [];
  const items = [];
  const pattern = /<li[^>]*version_list__item[^>]*>\s*([\s\S]*?)\s*<\/li>/g;
  let match;
  while ((match = pattern.exec(html.slice(listStart, listEnd))) !== null) {
    const text = decodeHtmlEntities(match[1].replace(/<[^>]*>/g, "")).trim();
    if (text) items.push(text);
  }
  return items;
}

/**
 * Extract a changelog block from a fixed download/log page (WPS Linux) and
 * convert it to Markdown.
 *
 * The block starts at `blockMarker` (an opening tag such as `<h2 class="log_title"`)
 * and runs to the next `</div>`. Tag attributes are normalized away (WPS emits
 * `<p class=…>`, `<strong style=…>`, `<ol class=…>`), `<br>` becomes a newline,
 * and empty `<strong>`/`<p>` separators are dropped before the shared
 * `htmlToMarkdown` converter runs.
 *
 * @param {unknown} html
 * @param {string} [blockMarker]
 * @returns {string}
 */
export function extractHtmlBlockToMarkdown(html, blockMarker) {
  if (typeof html !== "string" || typeof blockMarker !== "string" || !blockMarker) return "";
  const start = html.indexOf(blockMarker);
  if (start < 0) return "";
  const end = html.indexOf("</div>", start);
  if (end < 0) return "";
  let block = html.slice(start, end)
    .replace(/<(h2|h3|h4|p|strong|em|ol|ul|li|code|a|span|br)\b[^>]*>/gi, "<$1>")
    .replace(/<br\s*\/?>/gi, "\n")
    .replace(/<strong>\s*<\/strong>/g, "")
    .replace(/<p>\s*<\/p>/g, "");
  // The shared converter leaves the page's leading indentation in front of
  // headings (16 spaces would turn `**WPS 表格**` into a code block), so trim
  // each line's leading whitespace.
  return htmlToMarkdown(block).replace(/^[ \t]+/gm, "").trim();
}
