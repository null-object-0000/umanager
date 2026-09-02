import { describe, expect, it } from "vitest";
import { extractHtmlVersionSection, htmlChangelogToMarkdown, parseHtmlChangelog } from "./changelog-html.mjs";

describe("parseHtmlChangelog", () => {
  it("extracts h4 changelog bullets and decodes entities", () => {
    const html = '<div id="page_center"><p class="page_center_title">该版本主要更新如下：</p><h4>- 支持聊天记录导入与导出；</h4><h4>- 修复一些已知问题。</h4></div>';
    expect(parseHtmlChangelog(html)).toEqual([
      "- 支持聊天记录导入与导出；",
      "- 修复一些已知问题。",
    ]);
  });

  it("strips nested tags inside items", () => {
    const html = '<h4>- 修复了 <code>x</code> 与 &quot;y&quot; 的问题。</h4>';
    expect(parseHtmlChangelog(html)).toEqual(['- 修复了 x 与 "y" 的问题。']);
  });

  it("returns an empty array for non-strings", () => {
    expect(parseHtmlChangelog(null)).toEqual([]);
    expect(parseHtmlChangelog(42)).toEqual([]);
  });
});

describe("htmlChangelogToMarkdown", () => {
  it("joins items and normalizes a missing list marker", () => {
    expect(htmlChangelogToMarkdown(["- a", "b"])).toBe("- a\n- b");
  });

  it("returns empty string for empty input", () => {
    expect(htmlChangelogToMarkdown([])).toBe("");
    expect(htmlChangelogToMarkdown(null)).toBe("");
  });
});

describe("extractHtmlVersionSection", () => {
  const page = [
    "<h1 id=\"release-notes\">Release Notes</h1>",
    "<h2 id=\"history\">版本历史</h2>",
    '<h3 id="_4-11-2-2026-08-20" tabindex="-1">4.11.2 (2026-08-20) <a class="header-anchor" href="#_4-11-2-2026-08-20">​</a></h3>',
    "<p><strong>体验优化</strong></p>",
    "<ul><li>对话面板字体大小支持单独配置</li></ul>",
    "<p><strong>BUG修复</strong></p>",
    "<ul><li>修复 Agent 窗口文件删除未弹确认框问题</li><li>修复 ask_followup_question 参数异常</li></ul>",
    '<h3 id="_4-11-1-2026-08-14" tabindex="-1">4.11.1 (2026-08-14) <a class="header-anchor">​</a></h3>',
    "<ul><li>旧版本内容</li></ul>",
  ].join("");

  it("extracts the exact version section and converts it to Markdown", () => {
    const section = extractHtmlVersionSection(page, "4.11.1");
    expect(section).toContain("### 4.11.1");
    expect(section).toContain("- 旧版本内容");
    expect(section).not.toContain("4.11.2");
  });

  it("falls back to the first (latest) section when the version is absent", () => {
    const section = extractHtmlVersionSection(page, "9.9.9");
    expect(section).toContain("### 4.11.2");
    expect(section).toContain("**体验优化**");
    expect(section).toContain("- 修复 Agent 窗口文件删除未弹确认框问题");
  });

  it("drops the VitePress permalink anchor from the heading", () => {
    const section = extractHtmlVersionSection(page, "4.11.2");
    expect(section).not.toContain("header-anchor");
    expect(section).not.toContain("#_4-11-2");
  });

  it("returns empty string for non-strings", () => {
    expect(extractHtmlVersionSection(null, "1.0.0")).toBe("");
  });
});
