import { describe, expect, it } from "vitest";
import { readSortMode, sortSoftwareItems } from "./model";
import type { SortMode } from "./model";
import { summarizePackages } from "./model";
import type { ManagedPackage } from "./types";

const base: ManagedPackage = {
  packageName: "code",
  displayName: "Visual Studio Code",
  vendor: "Microsoft",
  installedVersion: "1.0",
  candidateVersion: "1.0",
  architecture: "amd64",
  sourceKind: "officialRepository",
  sourceUrl: "https://packages.microsoft.com/repos/code",
  updateState: "upToDate",
  homepage: null,
};

describe("summarizePackages", () => {
  it("counts official repositories and available updates", () => {
    const packages: ManagedPackage[] = [
      base,
      {
        ...base,
        packageName: "google-chrome-stable",
        updateState: "updateAvailable",
      },
      {
        ...base,
        packageName: "flclash",
        sourceKind: "localPackage",
        updateState: "unknown",
      },
      {
        ...base,
        packageName: "chatgpt",
        candidateVersion: null,
        updateState: "unknown",
      },
    ];

    expect(summarizePackages(packages)).toEqual({
      total: 4,
      updates: 1,
      repositories: 3,
      pendingRepositoryChecks: 1,
    });
  });
});

// 排序用例：只有部分条目带有「版本发布时间」，未知时间的条目必须恒定排在最后。
const entries = [
  { name: "微信", updatedAt: 1_700_000_000 },
  { name: "企业微信", updatedAt: null },
  { name: "Chrome", updatedAt: 1_600_000_000 },
  { name: "飞书", updatedAt: 1_800_000_000 },
  { name: "钉钉", updatedAt: null },
];
const names = (items: typeof entries) => items.map((item) => item.name);
const sortBy = (mode: SortMode) => names(sortSoftwareItems(entries, mode, (item) => item.updatedAt, (item) => item.name));

describe("sortSoftwareItems", () => {
  it("orders by version release time, newest first, unknown last", () => {
    expect(sortBy("updatedDesc")).toEqual(["飞书", "微信", "Chrome", "钉钉", "企业微信"]);
  });

  it("orders oldest first and still keeps unknown entries last", () => {
    expect(sortBy("updatedAsc")).toEqual(["Chrome", "微信", "飞书", "钉钉", "企业微信"]);
  });

  it("sorts by display name", () => {
    expect(sortBy("nameAsc")).toEqual(["钉钉", "飞书", "企业微信", "微信", "Chrome"]);
  });

  it("falls back to the name for equal timestamps", () => {
    const same = [{ name: "微信", updatedAt: 1 }, { name: "钉钉", updatedAt: 1 }, { name: "飞书", updatedAt: 1 }];
    expect(names(sortSoftwareItems(same, "updatedDesc", (item) => item.updatedAt, (item) => item.name))).toEqual(["钉钉", "飞书", "微信"]);
  });

  it("does not mutate the input array", () => {
    const input = [...entries];
    sortSoftwareItems(input, "updatedDesc", (item) => item.updatedAt, (item) => item.name);
    expect(names(input)).toEqual(names(entries));
  });
});

describe("readSortMode", () => {
  it("accepts known modes and falls back to the newest-first default", () => {
    expect(readSortMode("nameAsc")).toBe("nameAsc");
    expect(readSortMode("updatedAsc")).toBe("updatedAsc");
    expect(readSortMode("updatedDesc")).toBe("updatedDesc");
    expect(readSortMode(null)).toBe("updatedDesc");
    expect(readSortMode("sizeDesc")).toBe("updatedDesc");
  });
});
