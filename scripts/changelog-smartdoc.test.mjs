import { describe, expect, it } from "vitest";
import { parseSmartdocVersionSections, selectSmartdocSection, smartdocSectionToMarkdown, smartdocUpdateTimeToUnixSeconds } from "./changelog-smartdoc.mjs";

const PAGE_ID = "page-1";

// A trimmed mirror of the real Tencent Docs Linux changelog smartdoc payload.
function fixturePayload() {
  const block = (id, type, title, children) => ({
    [id]: { value: { type, props: { title }, children } },
  });
  return {
    result: {
      curPageData: {
        records: {
          block: {
            ...block("page-1", "page", [["腾讯文档"], ["Linux版本介绍"]], ["callout-1", "h-hist", "h-3117", "b-3117a", "h-3116", "b-3116a", "h-350", "b-350a"]),
            ...block("callout-1", "callout", [], ["h-top", "t-time", "t-label", "t-1", "t-2"]),
            ...block("h-top", "header2", [["版本号3.11.8"]]),
            ...block("t-time", "text", [["更新时间：2026年8月5日"]]),
            ...block("t-label", "text", [["本次更新："]]),
            ...block("t-1", "text", [["- 支持Markdown本地文档AI人机双写。"]]),
            ...block("t-2", "text", [["- 优化拖拽触发上传交互。"]]),
            ...block("h-hist", "header2", [["历史版本"]]),
            ...block("h-3117", "header4", [["【Linux】版本号3.11.7", [["b"]]]]),
            ...block("b-3117a", "text", [["- 支持本地Markdown文档打开、编辑。"]]),
            ...block("h-3116", "header4", [["【Linux】版本号3.11.6"]]),
            ...block("b-3116a", "text", [["- 修复sheet品类本地文档编辑bug。"]]),
            // 3.5.0's bullets are rich-text arrays with a leading "- " segment.
            ...block("h-350", "header4", [["【Linux】版本号3.5.0"]]),
            ...block("b-350a", "text", [["- "], ["全新腾讯文档客户端，让创作更轻松。"]]),
          },
        },
      },
    },
  };
}

describe("parseSmartdocVersionSections", () => {
  it("walks the page tree in order and groups bullets under version headings", () => {
    const sections = parseSmartdocVersionSections(fixturePayload(), PAGE_ID);
    expect(sections.map((section) => section.version)).toEqual(["3.11.8", "3.11.7", "3.11.6", "3.5.0"]);
    expect(sections[0]).toEqual({ version: "3.11.8", updateTime: "2026年8月5日", bullets: ["- 支持Markdown本地文档AI人机双写。", "- 优化拖拽触发上传交互。"] });
    expect(sections[1].updateTime).toBeNull();
    expect(sections[3].bullets).toEqual(["- 全新腾讯文档客户端，让创作更轻松。"]);
  });

  it("tolerates malformed payloads", () => {
    expect(parseSmartdocVersionSections(null, PAGE_ID)).toEqual([]);
    expect(parseSmartdocVersionSections({ result: {} }, PAGE_ID)).toEqual([]);
    expect(parseSmartdocVersionSections({ result: { curPageData: { records: { block: {} } } } }, PAGE_ID)).toEqual([]);
  });
});

describe("selectSmartdocSection", () => {
  it("selects the exact version section when present", () => {
    const sections = parseSmartdocVersionSections(fixturePayload(), PAGE_ID);
    const selected = selectSmartdocSection(sections, "3.11.6");
    expect(selected.version).toBe("3.11.6");
  });

  it("falls back to the first (latest) section for unknown versions", () => {
    const sections = parseSmartdocVersionSections(fixturePayload(), PAGE_ID);
    const selected = selectSmartdocSection(sections, "3.12.2");
    expect(selected.version).toBe("3.11.8");
  });

  it("returns null when no sections exist", () => {
    expect(selectSmartdocSection([], "3.12.2")).toBeNull();
    expect(selectSmartdocSection(null, "3.12.2")).toBeNull();
  });
});

describe("smartdocSectionToMarkdown", () => {
  it("renders the update time and bullets", () => {
    const sections = parseSmartdocVersionSections(fixturePayload(), PAGE_ID);
    const markdown = smartdocSectionToMarkdown(selectSmartdocSection(sections, "3.11.8"));
    expect(markdown).toContain("更新时间：2026年8月5日");
    expect(markdown).toContain("- 支持Markdown本地文档AI人机双写。");
  });

  it("omits the date line when absent and returns empty for bullet-less sections", () => {
    const sections = parseSmartdocVersionSections(fixturePayload(), PAGE_ID);
    expect(smartdocSectionToMarkdown(sections[1])).toBe("- 支持本地Markdown文档打开、编辑。");
    expect(smartdocSectionToMarkdown({ version: "1.0", bullets: [] })).toBe("");
  });
});

describe("smartdocUpdateTimeToUnixSeconds", () => {
  it("parses 年月日 into unix seconds", () => {
    expect(smartdocUpdateTimeToUnixSeconds("2026年8月5日")).toBe(Math.floor(Date.UTC(2026, 7, 5) / 1000));
    expect(smartdocUpdateTimeToUnixSeconds("2026-08-05")).toBeNull();
    expect(smartdocUpdateTimeToUnixSeconds(null)).toBeNull();
  });
});