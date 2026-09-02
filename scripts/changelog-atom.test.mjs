import { describe, expect, it } from "vitest";
import {
  atomChangelog,
  decodeXmlEntities,
  htmlToMarkdown,
  parseAtomEntries,
  selectAtomEntry,
} from "./changelog-atom.mjs";

describe("decodeXmlEntities", () => {
  it("decodes XML escapes back into raw HTML", () => {
    const input = "&lt;h2&gt;Hi&lt;/h2&gt;&lt;li&gt;a &amp;quot;b&amp;quot;&lt;/li&gt;";
    expect(decodeXmlEntities(input)).toBe("<h2>Hi</h2><li>a &quot;b&quot;</li>");
  });

  it("returns empty string for non-strings", () => {
    expect(decodeXmlEntities(null)).toBe("");
    expect(decodeXmlEntities(42)).toBe("");
  });
});

describe("parseAtomEntries", () => {
  const feed = `<?xml version="1.0"?>
<feed>
  <entry>
    <title>Obsidian 1.13.7 Desktop (Public)</title>
    <link href="https://obsidian.md/changelog/2026-08-12-desktop-v1.13.7/"/>
    <content type="html">&lt;h2&gt;No longer broken&lt;/h2&gt;&lt;ul&gt;&lt;li&gt;fix a&lt;/li&gt;&lt;/ul&gt;</content>
  </entry>
  <entry>
    <title>Obsidian 1.13.7 Mobile (Public)</title>
    <link href="https://obsidian.md/changelog/2026-08-12-mobile-v1.13.7/"/>
    <content type="html">&lt;p&gt;mobile only&lt;/p&gt;</content>
  </entry>
</feed>`;

  it("extracts title, link and decoded HTML content per entry", () => {
    const entries = parseAtomEntries(feed);
    expect(entries).toHaveLength(2);
    expect(entries[0].title).toBe("Obsidian 1.13.7 Desktop (Public)");
    expect(entries[0].link).toBe("https://obsidian.md/changelog/2026-08-12-desktop-v1.13.7/");
    expect(entries[0].html).toBe("<h2>No longer broken</h2><ul><li>fix a</li></ul>");
  });

  it("returns an empty array for non-strings", () => {
    expect(parseAtomEntries(null)).toEqual([]);
    expect(parseAtomEntries(123)).toEqual([]);
  });
});

describe("selectAtomEntry", () => {
  const entries = [
    { title: "Obsidian 1.13.8 Mobile (Public)" },
    { title: "Obsidian 1.13.7 Desktop (Public)" },
    { title: "Obsidian 1.13.7 Desktop (Early access)" },
  ];

  it("requires every filter substring to match", () => {
    expect(selectAtomEntry(entries, ["Desktop", "Public"]).title).toBe("Obsidian 1.13.7 Desktop (Public)");
  });

  it("skips entries missing any filter term", () => {
    expect(selectAtomEntry(entries, ["Desktop", "Early"]).title).toBe("Obsidian 1.13.7 Desktop (Early access)");
    expect(selectAtomEntry(entries, ["Desktop", "Nope"])).toBeNull();
  });

  it("returns the first entry when no filter is given", () => {
    expect(selectAtomEntry(entries, [])).toBe(entries[0]);
    expect(selectAtomEntry(entries)).toBe(entries[0]);
  });
});

describe("htmlToMarkdown", () => {
  it("converts headings, lists, code, links and emphasis", () => {
    const html = '<h2>New</h2><ul><li>Added <code>--foo</code> and <a href="https://x">docs</a></li></ul><p><strong>bold</strong> and <em>italic</em></p>';
    const md = htmlToMarkdown(html);
    expect(md).toContain("## New");
    expect(md).toContain("- Added `--foo` and [docs](https://x)");
    expect(md).toContain("**bold**");
    expect(md).toContain("*italic*");
  });

  it("drops media divs and iframes", () => {
    const html = '<p>before</p><div style="padding:1px"><iframe src="https://x/video"></iframe></div><p>after</p>';
    const md = htmlToMarkdown(html);
    expect(md).not.toContain("iframe");
    expect(md).not.toContain("video");
    expect(md).toContain("before");
    expect(md).toContain("after");
  });

  it("decodes residual HTML entities", () => {
    expect(htmlToMarkdown("<p>a &quot;quote&quot; &amp; b</p>")).toContain('a "quote" & b');
  });

  it("handles code blocks with attributes", () => {
    const html = '<pre><code class="language-ts">let x = 1;</code></pre>';
    expect(htmlToMarkdown(html)).toContain("```");
    expect(htmlToMarkdown(html)).toContain("let x = 1;");
  });
});

describe("atomChangelog", () => {
  it("returns sanitized-shaped release notes + https link for a matching entry", () => {
    const feed = `<feed><entry>
      <title>Obsidian 1.13.7 Desktop (Public)</title>
      <link href="https://obsidian.md/changelog/2026-08-12-desktop-v1.13.7/"/>
      <content type="html">&lt;h2&gt;New&lt;/h2&gt;&lt;ul&gt;&lt;li&gt;feature&lt;/li&gt;&lt;/ul&gt;</content>
    </entry></feed>`;
    const result = atomChangelog(feed, ["Desktop", "Public"]);
    expect(result.releaseNotesUrl).toBe("https://obsidian.md/changelog/2026-08-12-desktop-v1.13.7/");
    expect(result.releaseNotes).toContain("## New");
    expect(result.releaseNotes).toContain("- feature");
  });

  it("returns null when no entry matches", () => {
    const feed = `<feed><entry><title>Obsidian 1.13.7 Mobile (Public)</title><link href="https://obsidian.md/x"/><content type="html">&lt;p&gt;m&lt;/p&gt;</content></entry></feed>`;
    expect(atomChangelog(feed, ["Desktop", "Public"])).toBeNull();
  });
});
