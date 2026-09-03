import { describe, expect, it } from "vitest";
import {
  applyVersionTime,
  mergeSourceFeeds,
  parseCatalogApplications,
  sourceGroupOf,
  validateSourceGroups,
} from "./feed-merge.mjs";

const BUILTIN_GROUPS = { tencent: ["wechat", "wemeet"] };

describe("sourceGroupOf", () => {
  it("honors an explicit sourceGroup", () => {
    expect(sourceGroupOf({ applicationId: "qq", sourceGroup: "tencent" }, BUILTIN_GROUPS)).toBe("tencent");
  });

  it("maps built-in apps through the script-side table", () => {
    expect(sourceGroupOf({ applicationId: "wechat" }, BUILTIN_GROUPS)).toBe("tencent");
    expect(sourceGroupOf({ applicationId: "wemeet" }, BUILTIN_GROUPS)).toBe("tencent");
  });

  it("defaults everything else to common", () => {
    expect(sourceGroupOf({ applicationId: "vscode" }, BUILTIN_GROUPS)).toBe("common");
    expect(sourceGroupOf({ applicationId: "qq-music" }, BUILTIN_GROUPS)).toBe("common");
  });
});

describe("validateSourceGroups", () => {
  it("reports a typo'd sourceGroup loudly", () => {
    const problems = validateSourceGroups(
      [{ applicationId: "qq", sourceGroup: "tenceht" }],
      BUILTIN_GROUPS,
      ["tencent", "common"],
    );
    expect(problems).toHaveLength(1);
    expect(problems[0]).toContain("tenceht");
  });

  it("accepts a fully valid assignment set", () => {
    const problems = validateSourceGroups(
      [
        { applicationId: "qq", sourceGroup: "tencent" },
        { applicationId: "github-cli" },
        { applicationId: "wechat" },
      ],
      BUILTIN_GROUPS,
      ["tencent", "common"],
    );
    expect(problems).toEqual([]);
  });
});

describe("parseCatalogApplications", () => {
  it("parses a signed catalog blob into a map", () => {
    const map = parseCatalogApplications(
      JSON.stringify([{ applicationId: "qq", packageName: "linuxqq" }, { applicationId: "obsidian" }]),
    );
    expect(map.qq.packageName).toBe("linuxqq");
    expect(map.obsidian).toBeTruthy();
  });

  it("returns null for junk", () => {
    expect(parseCatalogApplications(null)).toBeNull();
    expect(parseCatalogApplications("not json")).toBeNull();
    expect(parseCatalogApplications(JSON.stringify({ not: "an array" }))).toBeNull();
  });
});

describe("applyVersionTime", () => {
  it("adopts an official candidate and strips it", () => {
    const entry = { version: "2.0", _versionTimeCandidate: { time: 5, source: "official" } };
    applyVersionTime(entry, { version: "1.0" }, 1000);
    expect(entry.versionUpdatedAtUnixSeconds).toBe(5);
    expect(entry.versionUpdatedAtSource).toBe("official");
    expect(entry._versionTimeCandidate).toBeUndefined();
  });

  it("records an observed upgrade when there is no candidate", () => {
    const entry = { version: "2.0" };
    applyVersionTime(entry, { version: "1.0" }, 1000);
    expect(entry.versionUpdatedAtUnixSeconds).toBe(1000);
    expect(entry.versionUpdatedAtSource).toBe("observed");
  });
});

function makeSourceFeed(group, { applications = {}, catalogJson } = {}) {
  return {
    group,
    applications,
    catalogJson: catalogJson ?? JSON.stringify([]),
  };
}

describe("mergeSourceFeeds", () => {
  const allApps = [
    { applicationId: "wechat" }, // built-in tencent
    { applicationId: "vscode" }, // built-in common
    { applicationId: "qq", sourceGroup: "tencent" }, // extra tencent
    { applicationId: "obsidian" }, // extra common
  ];
  const extraApps = [
    { applicationId: "qq", sourceGroup: "tencent" },
    { applicationId: "obsidian" },
  ];
  const previousFeed = {
    applications: {
      wechat: { version: "4.0", sha256: "prev-wechat" },
      vscode: { version: "1.89", sha256: "prev-vscode" },
      qq: { version: "3.1.1", sha256: "prev-qq" },
      obsidian: { version: "1.5", sha256: "prev-obsidian" },
    },
    catalogJson: JSON.stringify([
      { applicationId: "qq", packageName: "linuxqq", iconUrl: "https://x/icons/qq.png", iconSha256: "i-qq" },
      { applicationId: "obsidian", packageName: "obsidian", iconUrl: "https://x/icons/obsidian.png", iconSha256: "i-obs" },
    ]),
  };

  it("unions final source-feed entries in canonical app order", () => {
    const sourceFeeds = {
      tencent: makeSourceFeed("tencent", {
        applications: { qq: { version: "3.2.0", sha256: "fresh-qq" }, wechat: { version: "4.1" } },
        catalogJson: JSON.stringify([{ applicationId: "qq", packageName: "linuxqq" }]),
      }),
      common: makeSourceFeed("common", {
        applications: { obsidian: { version: "1.6", sha256: "fresh-obs" }, vscode: { version: "1.90" } },
        catalogJson: JSON.stringify([{ applicationId: "obsidian", packageName: "obsidian" }]),
      }),
    };
    const result = mergeSourceFeeds({ allApps, extraApps, sourceFeeds, previousFeed, builtinGroups: BUILTIN_GROUPS });
    expect(Object.keys(result.applications)).toEqual(["wechat", "vscode", "qq", "obsidian"]);
    expect(result.applications.qq.version).toBe("3.2.0");
    expect(result.applications.wechat.version).toBe("4.1");
    expect(result.applications.vscode.version).toBe("1.90");
    expect(result.applications.obsidian.version).toBe("1.6");
    expect(Object.keys(result.catalogApps)).toEqual(["qq", "obsidian"]);
  });

  it("falls back to the previous central feed when a source group is missing", () => {
    const sourceFeeds = {
      common: makeSourceFeed("common", {
        applications: { obsidian: { version: "1.6" } },
        catalogJson: JSON.stringify([{ applicationId: "obsidian", packageName: "obsidian" }]),
      }),
    };
    const result = mergeSourceFeeds({ allApps, extraApps, sourceFeeds, previousFeed, builtinGroups: BUILTIN_GROUPS });
    expect(result.applications.qq.version).toBe("3.1.1"); // from previous central
    expect(result.applications.wechat.version).toBe("4.0");
    expect(result.catalogApps.qq.iconUrl).toBe("https://x/icons/qq.png"); // previous catalog record
    expect(result.missingGroups).toEqual(["tencent"]);
    expect(result.reused.some((e) => e === "应用 wechat")).toBe(true);
    expect(result.reused.some((e) => e === "目录 qq")).toBe(true);
  });

  it("notes apps never seen in any feed", () => {
    const sourceFeeds = {
      tencent: makeSourceFeed("tencent", {}),
      common: makeSourceFeed("common", {}),
    };
    const result = mergeSourceFeeds({ allApps, extraApps, sourceFeeds, previousFeed: null, builtinGroups: BUILTIN_GROUPS });
    expect(result.applications).toEqual({});
    expect(result.notes.some((n) => n.includes("vscode"))).toBe(true);
    expect(result.catalogApps).toEqual({});
  });
});