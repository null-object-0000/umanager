import { describe, expect, it } from "vitest";
import { aiButtonLabel, aiMode, canRegenerate, formatCachedOrigin, looksEnglish, regenerateButtonLabel, shouldWarnMissingSummary } from "./changelogTranslation";

describe("looksEnglish", () => {
  it("detects Latin-only changelogs", () => {
    expect(looksEnglish("## v1.2.0\n- fix crash on startup")).toBe(true);
    expect(looksEnglish("")).toBe(false);
    expect(looksEnglish("1234 !!!")).toBe(false);
    expect(looksEnglish("## v1.2.0\n- 修复启动崩溃")).toBe(false);
    // 中英混排（已经有人翻译过）不算英文。
    expect(looksEnglish("- fix 崩溃")).toBe(false);
  });
});

describe("aiMode", () => {
  it("translates English changelogs and only summarizes every other language", () => {
    expect(aiMode("## v1.2.0\n- fix crash on startup")).toBe("translate");
    expect(aiMode("## v1.2.0\n- 修复启动崩溃")).toBe("summarize");
    // 中英混排只归纳，不把中文原文再翻译一遍。
    expect(aiMode("- fix 崩溃")).toBe("summarize");
    // 非拉丁文本（日文/韩文等）同样只归纳。
    expect(aiMode("## 1.2.0\n- クラッシュを修正")).toBe("summarize");
  });

  it("offers nothing without a changelog body", () => {
    expect(aiMode("")).toBeNull();
    expect(aiMode("   \n  ")).toBeNull();
    expect(aiMode(null)).toBeNull();
    expect(aiMode(undefined)).toBeNull();
  });
});

describe("aiButtonLabel", () => {
  it("labels the first action per mode", () => {
    expect(aiButtonLabel({ mode: "translate", busy: false, showing: false, hasResult: false })).toBe("翻译并总结");
    expect(aiButtonLabel({ mode: "summarize", busy: false, showing: false, hasResult: false })).toBe("归纳总结");
  });

  it("switches between the AI result and the original text once a result exists", () => {
    expect(aiButtonLabel({ mode: "translate", busy: false, showing: false, hasResult: true })).toBe("查看译文");
    expect(aiButtonLabel({ mode: "translate", busy: false, showing: true, hasResult: true })).toBe("查看原文");
    // 归纳模式下原文始终在下方，按钮只控制「更新重点」这一块。
    expect(aiButtonLabel({ mode: "summarize", busy: false, showing: false, hasResult: true })).toBe("展开总结");
    expect(aiButtonLabel({ mode: "summarize", busy: false, showing: true, hasResult: true })).toBe("收起总结");
  });

  it("shows progress while a request is in flight", () => {
    expect(aiButtonLabel({ mode: "translate", busy: true, showing: false, hasResult: false })).toBe("翻译中…");
    expect(aiButtonLabel({ mode: "summarize", busy: true, showing: false, hasResult: false })).toBe("归纳中…");
  });
});

describe("canRegenerate", () => {
  it("only offers a forced request while the AI result is on screen", () => {
    expect(canRegenerate({ mode: "translate", busy: false, showing: true, hasResult: true })).toBe(true);
    expect(canRegenerate({ mode: "summarize", busy: false, showing: true, hasResult: true })).toBe(true);
    expect(canRegenerate({ mode: "translate", busy: false, showing: false, hasResult: true })).toBe(false);
    expect(canRegenerate({ mode: "translate", busy: false, showing: true, hasResult: false })).toBe(false);
    expect(canRegenerate({ mode: "translate", busy: true, showing: true, hasResult: true })).toBe(false);
  });
});

describe("regenerateButtonLabel", () => {
  it("names the forced action per mode", () => {
    expect(regenerateButtonLabel("translate")).toBe("重新翻译");
    expect(regenerateButtonLabel("summarize")).toBe("重新归纳");
  });
});

describe("formatCachedOrigin", () => {
  it("names the time and model of the cached result", () => {
    expect(formatCachedOrigin("translate", "deepseek-chat", "2026/02/12 10:30")).toBe("已展示上次翻译结果 · 2026/02/12 10:30 · deepseek-chat");
    expect(formatCachedOrigin("summarize", "deepseek-chat", "2026/02/12 10:30")).toBe("已展示上次归纳结果 · 2026/02/12 10:30 · deepseek-chat");
  });

  it("drops missing pieces instead of leaving empty separators", () => {
    expect(formatCachedOrigin("translate", "", "2026/02/12 10:30")).toBe("已展示上次翻译结果 · 2026/02/12 10:30");
    expect(formatCachedOrigin("summarize", "  ", null)).toBe("已展示上次归纳结果");
  });
});

describe("shouldWarnMissingSummary", () => {
  it("warns only when a finished translation has no summary", () => {
    const shown = { mode: "translate" as const, busy: false, showing: true, hasResult: true };
    expect(shouldWarnMissingSummary(null, shown)).toBe(true);
    expect(shouldWarnMissingSummary("  ", shown)).toBe(true);
    expect(shouldWarnMissingSummary("- 修复崩溃", shown)).toBe(false);
    expect(shouldWarnMissingSummary(null, { ...shown, busy: true })).toBe(false);
    expect(shouldWarnMissingSummary(null, { ...shown, showing: false })).toBe(false);
  });

  it("never warns in summarize mode, where a missing summary is a failed request", () => {
    expect(shouldWarnMissingSummary(null, { mode: "summarize", busy: false, showing: true, hasResult: true })).toBe(false);
  });
});
