import { describe, expect, it } from "vitest";
import { buildChangelog, parseCommitSubject, selectPreviousTag } from "./release-changelog.mjs";

describe("parseCommitSubject", () => {
  it("groups conventional-commit prefixes", () => {
    expect(parseCommitSubject("feat(clipboard): 剪贴板历史")).toMatchObject({
      heading: "新功能",
      scope: "clipboard",
      subject: "剪贴板历史",
    });
    expect(parseCommitSubject("fix(self-update): 强制刷新 feed")).toMatchObject({
      heading: "问题修复",
      scope: "self-update",
    });
    expect(parseCommitSubject("refactor(store): 统一卸载流程")).toMatchObject({
      heading: "重构",
      scope: "store",
    });
    expect(parseCommitSubject("ci: 更新 release 流程")).toMatchObject({
      heading: "维护与构建",
      scope: null,
    });
  });

  it("falls back for release / unknown subjects", () => {
    expect(parseCommitSubject("release: v0.8.10").heading).toBe("其他变更");
    expect(parseCommitSubject("祝福语不打折").heading).toBe("其他变更");
  });

  it("handles empty and non-strings", () => {
    expect(parseCommitSubject("")).toEqual({ heading: "其他变更", scope: null, subject: "" });
    expect(parseCommitSubject(null).subject).toBe("");
  });
});

describe("buildChangelog", () => {
  it("groups by heading and scopes the subject", () => {
    const output = buildChangelog(
      [
        "feat(clipboard): 剪贴板历史",
        "fix(self-update): 强制刷新 feed",
        "feat(dev-tools): 新增 hermes 源",
      ].join("\n"),
      "0.8.10",
    );
    expect(output).toContain("## 0.8.10");
    expect(output).toContain("### 新功能");
    expect(output).toContain("`clipboard`：剪贴板历史");
    expect(output).toContain("`dev-tools`：新增 hermes 源");
    expect(output).toContain("### 问题修复");
    expect(output).toContain("`self-update`：强制刷新 feed");
  });

  it("emits a placeholder when empty", () => {
    expect(buildChangelog("", "0.8.10")).toContain("无提交记录");
  });
});

describe("selectPreviousTag", () => {
  it("picks the newest tag older than the released version", () => {
    const tags = ["v0.8.8", "v0.8.9", "v0.8.10", "v0.8.7"];
    expect(selectPreviousTag(tags, "0.8.10")).toBe("v0.8.9");
    expect(selectPreviousTag(tags, "0.8.11")).toBe("v0.8.10");
  });

  it("returns the newest tag when releasing the current tip", () => {
    expect(selectPreviousTag(["v0.8.8", "v0.8.9"])).toBe("v0.8.9");
  });

  it("returns null when no older tag or no parseable tags exist", () => {
    expect(selectPreviousTag(["v0.8.10"], "0.8.10")).toBeNull();
    expect(selectPreviousTag([], "0.8.10")).toBeNull();
    expect(selectPreviousTag(["not-a-tag"], "0.8.10")).toBeNull();
  });
});