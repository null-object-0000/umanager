import { describe, expect, it } from "vitest";
import {
  parseVersionHistoryJson,
  selectVersionHistoryEntry,
  versionHistoryEntryMatchesVersion,
  versionHistoryEntryToMarkdown,
  versionHistoryEntryToUnixSeconds,
} from "./changelog-version-history.mjs";

// A trimmed mirror of HexHub's `history-exact` response: newest first, with a
// beta hot update on top, the production hot updates below it and the latest
// *full* release (`5.1.9`) further down.
const SAMPLE = JSON.stringify({
  code: 0,
  data: {
    content: [
      {
        id: "253",
        clientType: "desktop",
        created: "1790594970429",
        description: "[🔥] 支持更多客户端主题</br>\n[🐛] 修复偶现问题</br>",
        env: "beta",
        hotUpdate: true,
        version: "543",
        versionName: "5.4.3",
      },
      {
        id: "252",
        clientType: "desktop",
        created: "1789552211289",
        description: "[✨] 优化设置页面交互体验</br>",
        env: "production",
        hotUpdate: true,
        version: "542",
        versionName: "5.4.2",
      },
      {
        id: "225",
        clientType: "desktop",
        created: "1780839719931",
        description:
          "<h5>数据库</h5>\n[🔥] SQL编辑器正式接入AI Agent能力，支持数据库智能分析与操作</br>\n<b>请重新登录账号</b></br>\n[🐛] 修复ClickHouse查询结果导出功能异常</br>",
        env: "production",
        hotUpdate: false,
        version: "519",
        versionName: "5.1.9",
      },
      {
        id: "88",
        clientType: "desktop",
        created: "1739969045000",
        description: "[✨] 初版</br>",
        env: "production",
        hotUpdate: false,
        version: "132",
        versionName: "2.3.2",
      },
    ],
  },
});

describe("parseVersionHistoryJson", () => {
  it("extracts the data.content list", () => {
    const entries = parseVersionHistoryJson(SAMPLE);
    expect(entries).toHaveLength(4);
    expect(entries[0].versionName).toBe("5.4.3");
    expect(entries[2].versionName).toBe("5.1.9");
  });

  it("returns null for non-JSON, missing data and non-string input", () => {
    expect(parseVersionHistoryJson("not json")).toBeNull();
    expect(parseVersionHistoryJson('{"code":0}')).toBeNull();
    expect(parseVersionHistoryJson('{"data":{"content":{}}}')).toBeNull();
    expect(parseVersionHistoryJson(null)).toBeNull();
    expect(parseVersionHistoryJson(42)).toBeNull();
  });
});

describe("selectVersionHistoryEntry", () => {
  const entries = parseVersionHistoryJson(SAMPLE);

  it("matches the resolved version exactly", () => {
    expect(selectVersionHistoryEntry(entries, "5.1.9").versionName).toBe("5.1.9");
    expect(selectVersionHistoryEntry(entries, "v5.1.9").versionName).toBe("5.1.9");
    // A Debian revision suffix must not defeat the match.
    expect(selectVersionHistoryEntry(entries, "5.1.9-1").versionName).toBe("5.1.9");
    // Nor must a Debian epoch (which would otherwise make normalizeVersion
    // match the epoch digits and fall through to the fallback).
    expect(selectVersionHistoryEntry(entries, "1:5.1.9").versionName).toBe("5.1.9");
  });

  it("prefers the production record when a beta shares the version", () => {
    const withBeta = [
      { env: "beta", hotUpdate: true, versionName: "5.4.0" },
      { env: "production", hotUpdate: false, versionName: "5.4.0" },
    ];
    expect(selectVersionHistoryEntry(withBeta, "5.4.0").env).toBe("production");
  });

  it("falls back to the newest production non-hot entry, never to a beta hot update", () => {
    expect(selectVersionHistoryEntry(entries, "9.9.9").versionName).toBe("5.1.9");
    expect(selectVersionHistoryEntry(entries, undefined).versionName).toBe("5.1.9");
  });

  it("returns null when nothing usable is left", () => {
    expect(selectVersionHistoryEntry(null)).toBeNull();
    expect(selectVersionHistoryEntry([])).toBeNull();
    expect(selectVersionHistoryEntry([null, "x"])).toBeNull();
    expect(selectVersionHistoryEntry([{ env: "beta", versionName: "1.0.0" }], "1.0.0").env).toBe("beta");
  });
});

describe("versionHistoryEntryMatchesVersion", () => {
  const entries = parseVersionHistoryJson(SAMPLE);

  it("is true only for the record of that exact version", () => {
    const exact = selectVersionHistoryEntry(entries, "5.1.9");
    expect(versionHistoryEntryMatchesVersion(exact, "5.1.9")).toBe(true);
    expect(versionHistoryEntryMatchesVersion(exact, "1:5.1.9-1")).toBe(true);
    // A fallback selection is a different version -> no official publish time.
    const fallback = selectVersionHistoryEntry(entries, "9.9.9");
    expect(fallback.versionName).toBe("5.1.9");
    expect(versionHistoryEntryMatchesVersion(fallback, "9.9.9")).toBe(false);
    expect(versionHistoryEntryMatchesVersion(null, "5.1.9")).toBe(false);
    expect(versionHistoryEntryMatchesVersion(exact, undefined)).toBe(false);
  });
});

describe("versionHistoryEntryToMarkdown", () => {
  it("turns </br> line breaks into bullets and keeps section headings", () => {
    const entry = selectVersionHistoryEntry(parseVersionHistoryJson(SAMPLE), "5.1.9");
    expect(versionHistoryEntryToMarkdown(entry)).toBe(
      [
        "### 数据库",
        "- [🔥] SQL编辑器正式接入AI Agent能力，支持数据库智能分析与操作",
        "- **请重新登录账号**",
        "- [🐛] 修复ClickHouse查询结果导出功能异常",
      ].join("\n"),
    );
  });

  it("keeps sections separated when a heading carries attributes", () => {
    const markdown = versionHistoryEntryToMarkdown({
      description: '<h5 class="x">通用</h5></br><b style="color:red">注意</b></br>修复问题',
    });
    expect(markdown).toBe(["### 通用", "- **注意**", "- 修复问题"].join("\n"));
  });

  it("renders nothing without a description", () => {
    expect(versionHistoryEntryToMarkdown({ versionName: "5.1.9" })).toBe("");
    expect(versionHistoryEntryToMarkdown({ description: "   " })).toBe("");
    expect(versionHistoryEntryToMarkdown(null)).toBe("");
  });
});

describe("versionHistoryEntryToUnixSeconds", () => {
  it("converts the vendor's epoch milliseconds", () => {
    expect(versionHistoryEntryToUnixSeconds({ created: "1780839719931" })).toBe(1780839719);
    expect(versionHistoryEntryToUnixSeconds({ created: 1780839719931 })).toBe(1780839719);
  });

  it("accepts plain epoch seconds", () => {
    expect(versionHistoryEntryToUnixSeconds({ created: 1780839719 })).toBe(1780839719);
  });

  it("returns null for missing or unusable values", () => {
    expect(versionHistoryEntryToUnixSeconds({})).toBeNull();
    expect(versionHistoryEntryToUnixSeconds({ created: "later" })).toBeNull();
    expect(versionHistoryEntryToUnixSeconds({ created: 0 })).toBeNull();
    expect(versionHistoryEntryToUnixSeconds(null)).toBeNull();
  });
});
