import { describe, expect, it } from "vitest";
import {
  jsonpEntryToMarkdown,
  parseJsonpChangelog,
  selectJsonpChangelogEntry,
} from "./changelog-jsonp.mjs";

const SAMPLE = `;(function(){var params= [{"version":"QQ Linux版 3.2.33","date":"2026-09-02 11:08:45","feature":["本次更新：","- 修复了一些已知问题。","近期更新：","- 新增「戳一戳」互动，让聊天更有趣。"]},{"version":"QQ Linux版 3.2.32","date":"2026-07-30 11:34:06","feature":["本次更新：","- 优化消息提醒能力。"]}];
typeof callback_1888_config1 === "function" && callback_1888_config1(params)
this.define && define(function (require, exports, module) { return params});
})()`;

describe("parseJsonpChangelog", () => {
  it("extracts the params array from a JSONP script", () => {
    const entries = parseJsonpChangelog(SAMPLE);
    expect(Array.isArray(entries)).toBe(true);
    expect(entries).toHaveLength(2);
    expect(entries[0].version).toBe("QQ Linux版 3.2.33");
  });

  it("returns null for non-strings and invalid scripts", () => {
    expect(parseJsonpChangelog(null)).toBeNull();
    expect(parseJsonpChangelog("no params here")).toBeNull();
  });
});

describe("selectJsonpChangelogEntry", () => {
  const entries = parseJsonpChangelog(SAMPLE);

  it("matches the numeric part of the version", () => {
    expect(selectJsonpChangelogEntry(entries, "3.2.32").version).toBe("QQ Linux版 3.2.32");
    expect(selectJsonpChangelogEntry(entries, "3.2.32-52194").version).toBe("QQ Linux版 3.2.32");
  });

  it("falls back to the first (latest) entry when the version is absent", () => {
    expect(selectJsonpChangelogEntry(entries, "9.9.9").version).toBe("QQ Linux版 3.2.33");
    expect(selectJsonpChangelogEntry(entries, undefined).version).toBe("QQ Linux版 3.2.33");
  });

  it("returns null for empty input", () => {
    expect(selectJsonpChangelogEntry([], "3.2.32")).toBeNull();
    expect(selectJsonpChangelogEntry(null, "3.2.32")).toBeNull();
  });
});

describe("jsonpEntryToMarkdown", () => {
  it("bolds section labels and keeps bullets as a list", () => {
    const entry = {
      version: "QQ Linux版 3.2.32",
      feature: ["本次更新：", "- 优化消息提醒能力。", "近期更新：", "- 新增「戳一戳」互动。"],
    };
    expect(jsonpEntryToMarkdown(entry)).toBe(
      "**本次更新：**\n- 优化消息提醒能力。\n\n**近期更新：**\n- 新增「戳一戳」互动。",
    );
  });

  it("returns empty string for entries without a feature list", () => {
    expect(jsonpEntryToMarkdown(null)).toBe("");
    expect(jsonpEntryToMarkdown({ version: "x" })).toBe("");
  });
});
