import { describe, expect, it } from "vitest";
import {
  jsonListEntryToMarkdown,
  parseJsonListChangelog,
  selectJsonListChangelogEntry,
} from "./changelog-json-list.mjs";

const SAMPLE = {
  errno: 0,
  list: [
    {
      detail: [
        { more: ["【体验优化】修复多项体验问题，持续打磨更流畅的使用体验"], stable: true, title: "百度网盘全新升级" },
      ],
      publish: "2026-08-05 20:27:00",
      size: "263.1M",
      system: "中标麒麟桌面操作系统软件（兆芯版） V7.0、Ubuntu V18.04、统信UOS V20",
      title: "百度网盘Linux电脑客户端V8.7.0",
      url: "https://pkg-ant.baidu.com/issue/netdisk/LinuxGuanjia/8.7.0/baidunetdisk-8.7.0.x86_64.rpm",
      url_1: "https://pkg-ant.baidu.com/issue/netdisk/LinuxGuanjia/8.7.0/baidunetdisk_8.7.0_amd64.deb",
      version: "百度网盘Linux电脑客户端V8.7.0",
    },
    {
      detail: [
        {
          more: [
            "【样式升级】布局更懂你，交互更随心。不止存储，更是您的贴心管家",
            "【一键养虾】一键云端部署OpenClaw，认养能做事、会学习、持续进化的AI伙伴",
            "【体验优化】优化了多项体验问题，我们始终致力于为您提供更优质的服务体验",
          ],
          stable: true,
          title: "更新内容：",
        },
      ],
      publish: "2026-06-08 23:47:00",
      size: "245.3M",
      title: "百度网盘Linux电脑客户端V8.5.2",
      url: "https://pkg-ant.baidu.com/issue/netdisk/LinuxGuanjia/8.5.2.427/baidunetdisk-8.5.2.x86_64.rpm",
      url_1: "https://pkg-ant.baidu.com/issue/netdisk/LinuxGuanjia/8.5.2.427/baidunetdisk_8.5.2_amd64.deb",
      version: "百度网盘Linux电脑客户端V8.5.2",
    },
  ],
  total: 19,
};

describe("parseJsonListChangelog", () => {
  it("extracts the list array from a JSON-list response", () => {
    const list = parseJsonListChangelog(JSON.stringify(SAMPLE));
    expect(Array.isArray(list)).toBe(true);
    expect(list).toHaveLength(2);
    expect(list[0].version).toBe("百度网盘Linux电脑客户端V8.7.0");
  });

  it("returns null for non-strings and invalid payloads", () => {
    expect(parseJsonListChangelog(null)).toBeNull();
    expect(parseJsonListChangelog("not json")).toBeNull();
    expect(parseJsonListChangelog('{"errno":0}')).toBeNull();
  });
});

describe("selectJsonListChangelogEntry", () => {
  const list = parseJsonListChangelog(JSON.stringify(SAMPLE));

  it("matches the numeric part of the version", () => {
    expect(selectJsonListChangelogEntry(list, "8.7.0").version).toBe("百度网盘Linux电脑客户端V8.7.0");
    expect(selectJsonListChangelogEntry(list, "8.5.2").version).toBe("百度网盘Linux电脑客户端V8.5.2");
  });

  it("falls back to the first (latest) entry when the version is absent", () => {
    expect(selectJsonListChangelogEntry(list, "9.9.9").version).toBe("百度网盘Linux电脑客户端V8.7.0");
    expect(selectJsonListChangelogEntry(list, undefined).version).toBe("百度网盘Linux电脑客户端V8.7.0");
  });

  it("returns null for empty input", () => {
    expect(selectJsonListChangelogEntry([], "8.7.0")).toBeNull();
    expect(selectJsonListChangelogEntry(null, "8.7.0")).toBeNull();
  });
});

describe("jsonListEntryToMarkdown", () => {
  it("bolds section titles and keeps more items as bullets", () => {
    const entry = SAMPLE.list[1];
    expect(jsonListEntryToMarkdown(entry)).toBe(
      "**更新内容：**\n- 【样式升级】布局更懂你，交互更随心。不止存储，更是您的贴心管家\n- 【一键养虾】一键云端部署OpenClaw，认养能做事、会学习、持续进化的AI伙伴\n- 【体验优化】优化了多项体验问题，我们始终致力于为您提供更优质的服务体验",
    );
  });

  it("renders a marketing heading without bullets", () => {
    expect(jsonListEntryToMarkdown(SAMPLE.list[0])).toBe(
      "**百度网盘全新升级**\n- 【体验优化】修复多项体验问题，持续打磨更流畅的使用体验",
    );
  });

  it("returns empty string for entries without a detail list", () => {
    expect(jsonListEntryToMarkdown(null)).toBe("");
    expect(jsonListEntryToMarkdown({ version: "x" })).toBe("");
    expect(jsonListEntryToMarkdown({ detail: [{ more: ["  "] }] })).toBe("");
  });
});
