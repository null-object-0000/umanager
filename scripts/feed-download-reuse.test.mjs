import { describe, expect, it } from "vitest";
import {
  decideDownloadSkip,
  digestProvesUnchanged,
  isSha256,
  parseContentRangeTotal,
  previousEntryIsReusable,
  summarizeSkipped,
} from "./feed-download-reuse.mjs";

const DIGEST = "a".repeat(64);
const PREVIOUS_DIGEST = "b".repeat(64);

function previous(overrides = {}) {
  return {
    version: "1.2.3",
    size: 1000,
    sha256: DIGEST,
    downloadUrl: "https://cdn.example.com/app_1.2.3_amd64.deb",
    websiteVersion: "1.2.3",
    ...overrides,
  };
}

describe("isSha256", () => {
  it("accepts only 64-character hex strings", () => {
    expect(isSha256(DIGEST)).toBe(true);
    expect(isSha256("A".repeat(64))).toBe(true);
    expect(isSha256("a".repeat(63))).toBe(false);
    expect(isSha256("z".repeat(64))).toBe(false);
    expect(isSha256("sha256:" + DIGEST)).toBe(false);
    expect(isSha256(null)).toBe(false);
    expect(isSha256(undefined)).toBe(false);
  });
});

describe("digestProvesUnchanged", () => {
  it("proves reuse when the vendor digest equals the previous entry's", () => {
    expect(digestProvesUnchanged(DIGEST, previous())).toBe(true);
  });

  it("is case-insensitive on both sides", () => {
    expect(digestProvesUnchanged(DIGEST.toUpperCase(), previous())).toBe(true);
    expect(digestProvesUnchanged(DIGEST, previous({ sha256: DIGEST.toUpperCase() }))).toBe(true);
  });

  it("never proves reuse without a valid digest on both sides", () => {
    expect(digestProvesUnchanged(PREVIOUS_DIGEST, previous())).toBe(false);
    expect(digestProvesUnchanged("", previous())).toBe(false);
    expect(digestProvesUnchanged(DIGEST, null)).toBe(false);
    expect(digestProvesUnchanged(DIGEST, previous({ sha256: "" }))).toBe(false);
    expect(digestProvesUnchanged(DIGEST, previous({ sha256: undefined }))).toBe(false);
  });
});

describe("previousEntryIsReusable", () => {
  it("accepts a complete entry", () => {
    expect(previousEntryIsReusable(previous())).toBe(true);
  });

  it.each([
    ["no entry", null],
    ["missing sha256", previous({ sha256: null })],
    ["missing size", previous({ size: undefined })],
    ["zero size", previous({ size: 0 })],
    ["missing version", previous({ version: "" })],
    ["missing downloadUrl", previous({ downloadUrl: null })],
  ])("rejects %s", (_label, entry) => {
    expect(previousEntryIsReusable(entry)).toBe(false);
  });
});

describe("parseContentRangeTotal", () => {
  it("reads the total from a Content-Range header", () => {
    expect(parseContentRangeTotal("bytes 0-0/316808988")).toBe(316808988);
    expect(parseContentRangeTotal("bytes 0-0 / 1234")).toBe(1234);
  });

  it("returns null for anything unusable", () => {
    expect(parseContentRangeTotal("bytes 0-0/*")).toBeNull();
    expect(parseContentRangeTotal("bytes 0-0/0")).toBeNull();
    expect(parseContentRangeTotal("")).toBeNull();
    expect(parseContentRangeTotal(null)).toBeNull();
  });
});

describe("decideDownloadSkip", () => {
  const rawUrl = "https://cdn.example.com/app_1.2.3_amd64.deb";
  const probe = { contentLength: 1000, lastModified: null };

  it("skips when the URL and the vendor-published version are unchanged", () => {
    expect(decideDownloadSkip({ previousEntry: previous(), rawUrl, websiteVersion: "1.2.3", probe }))
      .toEqual({ skip: true, reason: "unchanged" });
  });

  it("skips an immutable URL build even without a version string", () => {
    expect(
      decideDownloadSkip({
        previousEntry: previous({ websiteVersion: null }),
        rawUrl,
        probe,
        immutableDownloadUrl: true,
      }),
    ).toEqual({ skip: true, reason: "unchanged-immutable-url" });
  });

  it("skips a rotating signed URL when the vendor version is unchanged and the size matches", () => {
    expect(
      decideDownloadSkip({
        previousEntry: previous(),
        rawUrl: "https://cdn.example.com/app.deb?x-signature=rotated",
        websiteVersion: "1.2.3",
        probe,
        dynamicDownloadUrl: true,
      }),
    ).toEqual({ skip: true, reason: "unchanged-dynamic-url" });
  });

  it("skips when the CDN still reports the Last-Modified recorded in the feed", () => {
    expect(
      decideDownloadSkip({
        previousEntry: previous({
          websiteVersion: null,
          versionUpdatedAtUnixSeconds: 1788518507,
          versionUpdatedAtSource: "serverModified",
        }),
        rawUrl,
        probe: { contentLength: 1000, lastModified: 1788518507 },
      }),
    ).toEqual({ skip: true, reason: "unchanged-server-modified" });
  });

  it("ignores a Last-Modified that is newer than the recorded one", () => {
    expect(
      decideDownloadSkip({
        previousEntry: previous({
          websiteVersion: null,
          versionUpdatedAtUnixSeconds: 1788518507,
          versionUpdatedAtSource: "serverModified",
        }),
        rawUrl,
        probe: { contentLength: 1000, lastModified: 1788600000 },
      }),
    ).toEqual({ skip: false, reason: "version-unknown" });
  });

  it("ignores a recorded time that is not from the server Last-Modified probe", () => {
    expect(
      decideDownloadSkip({
        previousEntry: previous({
          websiteVersion: null,
          versionUpdatedAtUnixSeconds: 1788518507,
          versionUpdatedAtSource: "official",
        }),
        rawUrl,
        probe: { contentLength: 1000, lastModified: 1788518507 },
      }),
    ).toEqual({ skip: false, reason: "version-unknown" });
  });

  it("downloads when the vendor version changed", () => {
    expect(decideDownloadSkip({ previousEntry: previous(), rawUrl, websiteVersion: "1.2.4", probe }))
      .toEqual({ skip: false, reason: "version-changed" });
  });

  it("downloads when the download URL changed", () => {
    expect(
      decideDownloadSkip({
        previousEntry: previous(),
        rawUrl: "https://cdn.example.com/app_1.2.4_amd64.deb",
        websiteVersion: "1.2.4",
        probe,
      }),
    ).toEqual({ skip: false, reason: "url-changed" });
  });

  it("downloads when the size changed, even for an immutable URL", () => {
    expect(
      decideDownloadSkip({
        previousEntry: previous({ websiteVersion: null }),
        rawUrl,
        probe: { contentLength: 1001 },
        immutableDownloadUrl: true,
      }),
    ).toEqual({ skip: false, reason: "size-changed" });
  });

  it("downloads when no probe is available", () => {
    expect(decideDownloadSkip({ previousEntry: previous(), rawUrl, websiteVersion: "1.2.3", probe: null }))
      .toEqual({ skip: false, reason: "no-probe" });
    expect(
      decideDownloadSkip({ previousEntry: previous(), rawUrl, websiteVersion: "1.2.3", probe: { contentLength: 0 } }),
    ).toEqual({ skip: false, reason: "no-probe" });
  });

  it("downloads when there is no usable version signal and no opt-in", () => {
    expect(decideDownloadSkip({ previousEntry: previous({ websiteVersion: null }), rawUrl, probe }))
      .toEqual({ skip: false, reason: "version-unknown" });
    expect(decideDownloadSkip({ previousEntry: previous(), rawUrl, websiteVersion: null, probe }))
      .toEqual({ skip: false, reason: "version-unknown" });
  });

  it("downloads when there is no previous entry to reuse", () => {
    expect(decideDownloadSkip({ previousEntry: null, rawUrl, websiteVersion: "1.2.3", probe }))
      .toEqual({ skip: false, reason: "no-previous" });
  });

  it("downloads when the previous entry is incomplete", () => {
    expect(decideDownloadSkip({ previousEntry: previous({ sha256: null }), rawUrl, websiteVersion: "1.2.3", probe }))
      .toEqual({ skip: false, reason: "previous-incomplete" });
  });

  it("honours the per-source forceDownload opt-out", () => {
    expect(
      decideDownloadSkip({
        previousEntry: previous(),
        rawUrl,
        websiteVersion: "1.2.3",
        probe,
        forceDownload: true,
      }),
    ).toEqual({ skip: false, reason: "forced" });
  });

  it("treats a missing argument object as 'download'", () => {
    expect(decideDownloadSkip()).toEqual({ skip: false, reason: "no-previous" });
  });
});

describe("summarizeSkipped", () => {
  it("reports nothing skipped", () => {
    expect(summarizeSkipped([])).toBe("未跳过任何整包下载");
    expect(summarizeSkipped(undefined)).toBe("未跳过任何整包下载");
  });

  it("sums megabytes and groups reasons", () => {
    const summary = summarizeSkipped([
      { applicationId: "qq", size: 100 * 1024 * 1024, reason: "unchanged" },
      { applicationId: "obsidian", size: 24 * 1024 * 1024, reason: "unchanged-immutable-url" },
    ]);
    expect(summary).toContain("跳过 2 个");
    expect(summary).toContain("124 MB");
    expect(summary).toContain("unchanged×1");
  });

  it("switches to GB above 1024 MiB", () => {
    expect(summarizeSkipped([{ applicationId: "wps", size: 2048 * 1024 * 1024, reason: "unchanged" }]))
      .toContain("2.00 GB");
  });
});
