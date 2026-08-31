import { describe, expect, it } from "vitest";
import { debCategory, devToolCategory, orderedCategories, UNKNOWN_CATEGORY } from "./categories";
import type { CategoryCatalog } from "./types";

const catalog: CategoryCatalog = {
  categories: [
    { id: "dev-tools", label: "开发工具" },
    { id: "ai-tools", label: "AI 工具" },
    { id: "chat", label: "社交通讯" },
  ],
  assignments: {
    applications: { vscode: "dev-tools", wechat: "chat", "future-app": "unknown-id" },
    developmentTools: { codex: "ai-tools", "future-tool": "unknown-id" },
  },
};

describe("software categories", () => {
  it("maps .deb applications by applicationId from the built-in fallback", () => {
    expect(debCategory(null, "vscode")).toBe("开发工具");
    expect(debCategory(null, "github-cli")).toBe("开发工具");
    expect(debCategory(null, "chatgpt")).toBe("AI 工具");
    expect(debCategory(null, "google-chrome")).toBe("浏览器");
    expect(debCategory(null, "wechat")).toBe("社交通讯");
    expect(debCategory(null, "flclash")).toBe("网络工具");
    expect(debCategory(null, "bitwarden")).toBe("安全工具");
    expect(debCategory(null, "qq-music")).toBe("影音娱乐");
  });

  it("maps user-level CLI tools to AI 工具 from the built-in fallback", () => {
    expect(devToolCategory(null, "claude-code")).toBe("AI 工具");
    expect(devToolCategory(null, "opencode")).toBe("AI 工具");
    expect(devToolCategory(null, "pi")).toBe("AI 工具");
    expect(devToolCategory(null, "codex")).toBe("AI 工具");
    expect(devToolCategory(null, "dsh")).toBe("AI 工具");
  });

  it("prefers feed-driven categories and resolves labels from the feed", () => {
    expect(debCategory(catalog, "vscode")).toBe("开发工具");
    expect(debCategory(catalog, "wechat")).toBe("社交通讯");
    expect(devToolCategory(catalog, "codex")).toBe("AI 工具");
  });

  it("falls back to 其他 for unknown ids", () => {
    expect(debCategory(null, "future-app")).toBe(UNKNOWN_CATEGORY);
    expect(debCategory(null, null)).toBe(UNKNOWN_CATEGORY);
    expect(devToolCategory(null, "future-tool")).toBe(UNKNOWN_CATEGORY);
    expect(debCategory(catalog, "future-app")).toBe(UNKNOWN_CATEGORY);
  });

  it("orders present categories by the feed order and appends 其他 last", () => {
    expect(orderedCategories(catalog, new Set(["社交通讯", "开发工具", "未来分类"]))).toEqual([
      "开发工具",
      "社交通讯",
      UNKNOWN_CATEGORY,
    ]);
    expect(orderedCategories(null, new Set(["影音娱乐", "开发工具"]))).toEqual(["开发工具", "影音娱乐"]);
  });
});
