import { describe, expect, it } from "vitest";
import { canRetranslate, formatCachedOrigin, looksEnglish, shouldWarnMissingSummary, translateButtonLabel } from "./changelogTranslation";

describe("looksEnglish", () => {
  it("offers translation only for Latin-only changelogs", () => {
    expect(looksEnglish("## v1.2.0\n- fix crash on startup")).toBe(true);
    expect(looksEnglish("")).toBe(false);
    expect(looksEnglish("1234 !!!")).toBe(false);
    expect(looksEnglish("## v1.2.0\n- 修复启动崩溃")).toBe(false);
    // 中英混排（已经有人翻译过）不提供翻译。
    expect(looksEnglish("- fix 崩溃")).toBe(false);
  });
});

describe("translateButtonLabel", () => {
  it("labels the first action as translate + summarize", () => {
    expect(translateButtonLabel({ translating: false, showTranslated: false, hasTranslation: false })).toBe("翻译并总结");
  });

  it("switches between translation and the original text once a result exists", () => {
    expect(translateButtonLabel({ translating: false, showTranslated: false, hasTranslation: true })).toBe("查看译文");
    expect(translateButtonLabel({ translating: false, showTranslated: true, hasTranslation: true })).toBe("查看原文");
  });

  it("shows progress while a request is in flight", () => {
    expect(translateButtonLabel({ translating: true, showTranslated: false, hasTranslation: false })).toBe("翻译中…");
  });
});

describe("canRetranslate", () => {
  it("only offers a forced re-translation while the translation is on screen", () => {
    expect(canRetranslate({ translating: false, showTranslated: true, hasTranslation: true })).toBe(true);
    expect(canRetranslate({ translating: false, showTranslated: false, hasTranslation: true })).toBe(false);
    expect(canRetranslate({ translating: false, showTranslated: true, hasTranslation: false })).toBe(false);
    expect(canRetranslate({ translating: true, showTranslated: true, hasTranslation: true })).toBe(false);
  });
});

describe("formatCachedOrigin", () => {
  it("names the time and model of the cached result", () => {
    expect(formatCachedOrigin("deepseek-chat", "2026/02/12 10:30")).toBe("已展示上次翻译结果 · 2026/02/12 10:30 · deepseek-chat");
  });

  it("drops missing pieces instead of leaving empty separators", () => {
    expect(formatCachedOrigin("", "2026/02/12 10:30")).toBe("已展示上次翻译结果 · 2026/02/12 10:30");
    expect(formatCachedOrigin("  ", null)).toBe("已展示上次翻译结果");
  });
});

describe("shouldWarnMissingSummary", () => {
  it("warns only when a finished translation has no summary", () => {
    const shown = { translating: false, showTranslated: true, hasTranslation: true };
    expect(shouldWarnMissingSummary(null, shown)).toBe(true);
    expect(shouldWarnMissingSummary("  ", shown)).toBe(true);
    expect(shouldWarnMissingSummary("- 修复崩溃", shown)).toBe(false);
    expect(shouldWarnMissingSummary(null, { ...shown, translating: true })).toBe(false);
    expect(shouldWarnMissingSummary(null, { ...shown, showTranslated: false })).toBe(false);
  });
});
