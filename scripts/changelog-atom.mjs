// Atom-feed changelog parsing for the CI feed generator.
//
// Some vendors publish release notes only as an Atom feed of HTML fragments
// (Obsidian is the canonical case: its GitHub release body is a one-line link,
// but https://obsidian.md/changelog.xml carries the full per-version changelog
// as `<content type="html">`). This module turns that feed into the same
// `{ releaseNotes, releaseNotesUrl }` shape the rest of the generator emits.
//
// All parsing happens in CI; the desktop app only ever reads the signed feed.

const XML_ENTITIES = [
  ["&lt;", "<"],
  ["&gt;", ">"],
  ["&quot;", '"'],
  ["&apos;", "'"],
  ["&#39;", "'"],
  ["&#x27;", "'"],
  ["&amp;", "&"],
];

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

function decodeWith(text, entities) {
  let output = text;
  for (const [from, to] of entities) output = output.split(from).join(to);
  return output;
}

/** Decode XML character/entity references into the raw HTML fragment. */
export function decodeXmlEntities(text) {
  if (typeof text !== "string") return "";
  return decodeWith(text, XML_ENTITIES);
}

/**
 * Split an Atom feed body into entries: `{ title, link, html }` per `<entry>`.
 * The `html` is the entity-decoded `<content type="html">` body.
 */
export function parseAtomEntries(xml) {
  if (typeof xml !== "string") return [];
  const entries = [];
  const entryPattern = /<entry>([\s\S]*?)<\/entry>/g;
  let match;
  while ((match = entryPattern.exec(xml)) !== null) {
    const block = match[1];
    // Blogger-style feeds (Chrome Releases) emit `<title type="text">`; the
    // attribute form must match too.
    const title = firstCapture(/<title[^>]*>([\s\S]*?)<\/title>/, block) ?? "";
    const link = entryLink(block);
    const content = firstCapture(/<content[^>]*>([\s\S]*?)<\/content>/, block) ?? "";
    if (title) {
      entries.push({ title: title.trim(), link, html: decodeXmlEntities(content) });
    }
  }
  return entries;
}

function firstCapture(pattern, text) {
  const match = pattern.exec(text);
  return match ? match[1] : null;
}

// Return the href of the entry's canonical `rel="alternate"` link, else the
// first `<link>` href. Blogger emits `href` before `rel`, so a single ordered
// regex is not enough.
function entryLink(block) {
  const tags = block.match(/<link\b[^>]*>/g) ?? [];
  for (const tag of tags) {
    if (/\brel="alternate"/.test(tag)) {
      const href = firstCapture(/href="([^"]+)"/, tag);
      if (href) return href;
    }
  }
  for (const tag of tags) {
    const href = firstCapture(/href="([^"]+)"/, tag);
    if (href) return href;
  }
  return "";
}

/**
 * Pick the first feed entry whose title contains every substring in
 * `titleContains`. Empty/absent filter returns the first entry.
 */
export function selectAtomEntry(entries, titleContains) {
  const filters = (titleContains || []).filter((item) => typeof item === "string" && item);
  return (
    entries.find((entry) => filters.every((needle) => entry.title.includes(needle))) ?? null
  );
}

/**
 * Convert the small HTML subset Obsidian's changelog uses (h2/h3, p, ul/ol/li,
 * code, pre>code, strong/em, a, br) into Markdown. Media embeds (iframe/div)
 * are dropped, and any residual tags are stripped.
 */
export function htmlToMarkdown(html) {
  if (typeof html !== "string") return "";
  let text = html;

  // Drop media wrappers + iframes wholesale (videos embedded in release notes).
  text = text.replace(/<div[^>]*>[\s\S]*?<\/div>/g, "");
  text = text.replace(/<iframe[^>]*>[\s\S]*?<\/iframe>/g, "");

  // Google Docs-style export (Chrome Releases) decorates nearly every tag with
  // `style=…`/`dir=…`. Normalize those attributes away — except `<a href>` — so
  // the block/inline rules below recognize `<h2>`, `<p>`, `<li>`, `<span>` …
  text = text.replace(/<(h1|h2|h3|h4|p|strong|em|ul|ol|li|code|pre|br|span|b)\b[^>]*>/gi, "<$1>");

  // Code blocks before inline code so `<pre><code class>` wins.
  text = text.replace(/<pre><code[^>]*>([\s\S]*?)<\/code><\/pre>/g, "\n```\n$1\n```\n");

  // Block elements.
  text = text.replace(/<h2>([\s\S]*?)<\/h2>/g, "\n## $1\n");
  text = text.replace(/<h3>([\s\S]*?)<\/h3>/g, "\n### $1\n");
  text = text.replace(/<p>([\s\S]*?)<\/p>/g, "$1\n");
  text = text.replace(/<(ul|ol)[^>]*>/g, "\n");
  text = text.replace(/<\/(ul|ol)>/g, "\n");
  text = text.replace(/<li>([\s\S]*?)<\/li>/g, "- $1\n");

  // Inline elements.
  text = text.replace(/<code>([\s\S]*?)<\/code>/g, "`$1`");
  text = text.replace(/<strong>([\s\S]*?)<\/strong>/g, "**$1**");
  text = text.replace(/<em>([\s\S]*?)<\/em>/g, "*$1*");
  text = text.replace(/<a href="([^"]+)"[^>]*>([\s\S]*?)<\/a>/g, "[$2]($1)");
  text = text.replace(/<br\s*\/?>/g, "\n");

  // Strip anything we did not recognize, then decode remaining HTML entities.
  text = text.replace(/<[^>]+>/g, "");
  text = decodeWith(text, HTML_ENTITIES);

  return text.replace(/\n{3,}/g, "\n\n").trim();
}

/**
 * Parse an Atom changelog feed and produce `{ releaseNotes, releaseNotesUrl }`
 * for the matching entry, or null when nothing matches.
 */
export function atomChangelog(xml, titleContains) {
  const entry = selectAtomEntry(parseAtomEntries(xml), titleContains);
  if (!entry) return null;
  // Chrome's Blogger feed links entries over `http://` even though the site is
  // served over HTTPS; the app only opens https links, so upgrade the scheme.
  const link = typeof entry.link === "string" ? entry.link.replace(/^http:/, "https:") : entry.link;
  const releaseNotesUrl = /^https:\/\//.test(link) ? link : null;
  const releaseNotes = htmlToMarkdown(entry.html);
  if (!releaseNotes && !releaseNotesUrl) return null;
  return { releaseNotes, releaseNotesUrl };
}
