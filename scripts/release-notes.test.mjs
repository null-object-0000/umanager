import { describe, expect, it } from "vitest";
import {
  MAX_RELEASE_NOTES_BYTES,
  sanitizeReleaseNotes,
  selectReleaseNotesRelease,
  selectToolRelease,
  stripReleaseNotesBoilerplate,
} from "./release-notes.mjs";

describe("sanitizeReleaseNotes", () => {
  it("returns null for non-strings", () => {
    expect(sanitizeReleaseNotes(null)).toBeNull();
    expect(sanitizeReleaseNotes(undefined)).toBeNull();
    expect(sanitizeReleaseNotes(42)).toBeNull();
  });

  it("returns null for empty / whitespace / NUL-only bodies", () => {
    expect(sanitizeReleaseNotes("")).toBeNull();
    expect(sanitizeReleaseNotes("   \n\t ")).toBeNull();
    expect(sanitizeReleaseNotes("\0\0")).toBeNull();
  });

  it("trims and drops NUL bytes", () => {
    expect(sanitizeReleaseNotes("  ## hi\n\n- a\0- b  ")).toBe("## hi\n\n- a- b");
  });

  it("returns short bodies unchanged", () => {
    const body = "## 0.8.96\n\n- 修复托盘\n- 升级内核";
    expect(sanitizeReleaseNotes(body)).toBe(body);
  });

  it("truncates over-limit bodies and appends a suffix within the byte ceiling", () => {
    const body = "a".repeat(MAX_RELEASE_NOTES_BYTES + 5000);
    const result = sanitizeReleaseNotes(body);
    expect(Buffer.byteLength(result, "utf8")).toBeLessThanOrEqual(MAX_RELEASE_NOTES_BYTES);
    expect(result).toContain("…（内容过长，已截断，完整内容见发布页）");
  });

  it("does not split a multi-byte UTF-8 sequence at the cut", () => {
    // A run of 3-byte CJK characters: truncation must never emit a lone
    // continuation byte that would decode into U+FFFD.
    const body = "汉".repeat(MAX_RELEASE_NOTES_BYTES);
    const result = sanitizeReleaseNotes(body);
    expect(result).not.toContain("\uFFFD");
    expect(Buffer.byteLength(result, "utf8")).toBeLessThanOrEqual(MAX_RELEASE_NOTES_BYTES);
  });
});

describe("stripReleaseNotesBoilerplate", () => {
  const body = [
    "- Fix a thing",
    "- Optimize another thing",
    "",
    "<div align=center>",
    "",
    "[![Release Downloads](https://img.shields.io/badge/x-y.svg)](https://example.com)",
    "",
    "</div>",
    "",
    "**Download based on your OS:**",
    "",
    "<div align=left>",
    "<table>",
    "    <thead align=left>",
    "        <tr><th>OS</th><th>Download</th></tr>",
    "    </thead>",
    "    <tbody align=left>",
    "        <tr><td>Android</td><td><a href=\"https://example.com/a.apk\"><img src=\"https://img.shields.io/badge/APK-x.svg\"></a></td></tr>",
    "        <tr><td>Linux</td><td><a href=\"https://example.com/a.deb\"><img src=\"https://img.shields.io/badge/DebPackage-x.svg\"></a></td></tr>",
    "    </tbody>",
    "</table>",
    "</div>",
    "",
    "<div dir=\"ltr\">",
    "",
    "**List of all changes:** [ChangeLog](https://example.com/CHANGELOG.md)",
    "",
    "</div>",
  ].join("\n");

  it("strips the fixed download-matrix + changelog footer for flclash", () => {
    const result = stripReleaseNotesBoilerplate(body, "flclash");
    expect(result).not.toContain("Download based on your OS");
    expect(result).not.toContain("List of all changes");
    expect(result).not.toContain("Android");
    expect(result).toContain("- Fix a thing");
    expect(result).toContain("- Optimize another thing");
  });

  it("leaves other apps' bodies untouched", () => {
    expect(stripReleaseNotesBoilerplate(body, "localsend")).toBe(body);
  });

  it("leaves non-string input untouched", () => {
    expect(stripReleaseNotesBoilerplate(null, "flclash")).toBeNull();
    expect(stripReleaseNotesBoilerplate(undefined, "flclash")).toBeUndefined();
  });
});

describe("selectReleaseNotesRelease", () => {
  it("accepts a single release object (releases/latest)", () => {
    const release = { tag_name: "v1.0.0", body: "hi", draft: false, prerelease: false };
    expect(selectReleaseNotesRelease(release)).toBe(release);
  });

  it("skips drafts and prereleases", () => {
    const draft = { tag_name: "v1.0.0", draft: true, prerelease: false };
    const pre = { tag_name: "v1.0.0", draft: false, prerelease: true };
    expect(selectReleaseNotesRelease(draft)).toBeNull();
    expect(selectReleaseNotesRelease(pre)).toBeNull();
  });

  it("picks the first array release matching a tag prefix", () => {
    const releases = [
      { tag_name: "web-v2", draft: false, prerelease: false },
      { tag_name: "desktop-v1", draft: false, prerelease: false },
      { tag_name: "cli-v3", draft: false, prerelease: false },
    ];
    expect(selectReleaseNotesRelease(releases, "desktop-")).toEqual({ tag_name: "desktop-v1", draft: false, prerelease: false });
  });

  it("returns null when no array entry matches the prefix", () => {
    const releases = [
      { tag_name: "web-v2", draft: false, prerelease: false },
    ];
    expect(selectReleaseNotesRelease(releases, "desktop-")).toBeNull();
  });

  it("tolerates null / non-object entries and returns null on empty", () => {
    expect(selectReleaseNotesRelease(null)).toBeNull();
    expect(selectReleaseNotesRelease([])).toBeNull();
    expect(selectReleaseNotesRelease([null, 42, { tag_name: "ok", draft: false, prerelease: false }])).toEqual({ tag_name: "ok", draft: false, prerelease: false });
  });
});

describe("selectToolRelease", () => {
  it("matches the exact tag, allowing prereleases for rc versions", () => {
    const releases = [
      { tag_name: "dsh-v0.1.2-alpha.5", draft: false, prerelease: true },
      { tag_name: "dsh-v0.1.1-rc.2", draft: false, prerelease: true },
      { tag_name: "dsh-v0.1.0", draft: false, prerelease: false },
    ];
    expect(selectToolRelease(releases, "dsh-v", "0.1.1-rc.2")).toEqual({ tag_name: "dsh-v0.1.1-rc.2", draft: false, prerelease: true });
  });

  it("skips drafts even on exact match", () => {
    const releases = [
      { tag_name: "v1.0.0", draft: true, prerelease: false },
    ];
    expect(selectToolRelease(releases, "v", "1.0.0")).toBeNull();
  });

  it("falls back to the first stable release under the prefix when no exact tag matches", () => {
    const releases = [
      { tag_name: "rust-v0.152.1", draft: false, prerelease: false },
      { tag_name: "rust-v0.152.0", draft: false, prerelease: false },
    ];
    expect(selectToolRelease(releases, "rust-v", "0.151.0")).toEqual({ tag_name: "rust-v0.152.1", draft: false, prerelease: false });
  });

  it("returns null for empty input", () => {
    expect(selectToolRelease(null, "v", "1.0.0")).toBeNull();
    expect(selectToolRelease([], "v", "1.0.0")).toBeNull();
  });
});
