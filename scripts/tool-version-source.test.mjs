import { describe, expect, it } from "vitest";
import { resolveUpdaterManifest, releaseCommit } from "./tool-version-source.mjs";

const platforms = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"];
const release = (version, targets = platforms) => ({ version, date: "2026-10-20T15:24:59Z", artifacts: targets.map((platform) => ({ platform })) });
const manifest = (...entries) => entries.map(JSON.stringify).join("\n");

describe("official updater version sources", () => {
  it("selects a stable version available on every supported Linux platform", () => {
    const text = manifest(release("0.14.0-rc.1"), release("0.13.5", [platforms[0]]), release("0.13.4"));
    expect(resolveUpdaterManifest(text, platforms).version).toBe("0.13.4");
  });
  it("rejects unavailable or malformed metadata instead of guessing from releases", () => {
    expect(() => resolveUpdaterManifest(manifest(release("0.13.5", [])), platforms)).toThrow();
    expect(() => resolveUpdaterManifest('{broken', platforms)).toThrow();
    expect(() => resolveUpdaterManifest(manifest({ ...release("0.13.5"), date: "bad" }), platforms)).toThrow();
  });
  it("requires an exact resolved commit rather than a branch or short SHA", () => {
    expect(releaseCommit({ sha: "a".repeat(40) })).toBe("a".repeat(40));
    for (const sha of ["main", "a".repeat(12), "z".repeat(40), null]) {
      expect(() => releaseCommit({ sha })).toThrow();
    }
  });
});
