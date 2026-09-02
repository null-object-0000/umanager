import { describe, expect, it } from "vitest";
import {
  cleanReleaseNotesMarkdown,
  extractMarkdownVersionSection,
  stripHtmlComments,
  stripMarkdownFrontMatter,
  stripRawHtmlTags,
} from "./changelog-markdown.mjs";

describe("stripMarkdownFrontMatter", () => {
  it("removes a leading YAML front-matter block", () => {
    const input = "---\nOrder: 110\nTitle: x\n---\n# Heading\n\nbody";
    expect(stripMarkdownFrontMatter(input)).toBe("# Heading\n\nbody");
  });

  it("leaves a body with no front matter untouched", () => {
    expect(stripMarkdownFrontMatter("# Hi\n")).toBe("# Hi\n");
  });

  it("returns empty string for non-strings", () => {
    expect(stripMarkdownFrontMatter(null)).toBe("");
    expect(stripMarkdownFrontMatter(42)).toBe("");
  });
});

describe("stripHtmlComments", () => {
  it("removes inline and block HTML comments", () => {
    const input = "a <!-- x --> b\n<!-- TOC\n<div>x</div>\nEnd -->\nc";
    expect(stripHtmlComments(input)).toBe("a  b\n\nc");
  });
});

describe("stripRawHtmlTags", () => {
  it("removes raw HTML tags", () => {
    expect(stripRawHtmlTags('x <video src="a.mp4"></video> y <a id="t"></a>')).toBe("x  y ");
  });

  it("does not touch markdown image/link syntax", () => {
    expect(stripRawHtmlTags("![alt](img.png) [t](https://x)")).toBe("![alt](img.png) [t](https://x)");
  });
});

describe("cleanReleaseNotesMarkdown", () => {
  it("applies front-matter, comment and tag stripping in order", () => {
    const input = "---\nOrder: 1\n---\n# 1.135\n\n<!-- TOC\n<div>nav</div>\nEnd -->\n\n## Agents\n\n- item <video src=\"x.mp4\"></video>";
    const out = cleanReleaseNotesMarkdown(input);
    expect(out).toContain("# 1.135");
    expect(out).toContain("## Agents");
    expect(out).toContain("- item");
    expect(out).not.toContain("Order:");
    expect(out).not.toContain("<video");
    expect(out).not.toContain("<div>nav</div>");
  });

  it("returns empty string for non-strings", () => {
    expect(cleanReleaseNotesMarkdown(undefined)).toBe("");
  });
});

describe("extractMarkdownVersionSection", () => {
  const changelog = [
    "# Changelog",
    "",
    "## 2.1.258",
    "- fixed A",
    "- fixed B",
    "",
    "## 2.1.257",
    "- added X",
    "",
    "## 2.1.256",
    "- added Y",
  ].join("\n");

  it("extracts the exact version section", () => {
    const section = extractMarkdownVersionSection(changelog, "2.1.257");
    expect(section).toBe("## 2.1.257\n- added X");
  });

  it("falls back to the first section when the version is not found", () => {
    const section = extractMarkdownVersionSection(changelog, "9.9.9");
    expect(section).toBe("## 2.1.258\n- fixed A\n- fixed B");
  });

  it("returns empty string for non-strings", () => {
    expect(extractMarkdownVersionSection(null, "1.0.0")).toBe("");
  });
});
