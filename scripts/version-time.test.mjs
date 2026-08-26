import { describe, expect, it } from "vitest";
import {
  mergeVersionUpdatedAt,
  parseLastModified,
  parseUnixSeconds,
} from "./version-time.mjs";

describe("parseUnixSeconds", () => {
  it("parses ISO 8601 strings", () => {
    expect(parseUnixSeconds("2026-08-26T03:00:00Z")).toBe(Math.floor(Date.parse("2026-08-26T03:00:00Z") / 1000));
  });
  it("parses numeric strings and numbers", () => {
    expect(parseUnixSeconds("1750000000")).toBe(1750000000);
    expect(parseUnixSeconds(1750000000)).toBe(1750000000);
  });
  it("returns null on garbage / zero / negative", () => {
    expect(parseUnixSeconds(null)).toBeNull();
    expect(parseUnixSeconds(undefined)).toBeNull();
    expect(parseUnixSeconds("not a date")).toBeNull();
    expect(parseUnixSeconds(0)).toBeNull();
    expect(parseUnixSeconds(-1)).toBeNull();
  });
});

describe("parseLastModified", () => {
  it("parses an HTTP date header", () => {
    expect(parseLastModified("Wed, 21 Oct 2015 07:28:00 GMT")).toBe(1445412480);
  });
  it("returns null when absent or invalid", () => {
    expect(parseLastModified(null)).toBeNull();
    expect(parseLastModified("")).toBeNull();
    expect(parseLastModified("bogus")).toBeNull();
  });
});

describe("mergeVersionUpdatedAt", () => {
  const now = 1750000000;

  it("first scrape with no candidate -> null", () => {
    expect(mergeVersionUpdatedAt(null, "1.0", null, now)).toBeNull();
  });

  it("detected upgrade -> observed = now", () => {
    const previous = { version: "1.0", versionUpdatedAtUnixSeconds: null, versionUpdatedAtSource: null };
    expect(mergeVersionUpdatedAt(previous, "2.0", null, now)).toEqual({ time: now, source: "observed" });
  });

  it("unchanged with no previous value -> null", () => {
    const previous = { version: "1.0", versionUpdatedAtUnixSeconds: null, versionUpdatedAtSource: null };
    expect(mergeVersionUpdatedAt(previous, "1.0", null, now)).toBeNull();
  });

  it("unchanged with previous value -> carries it", () => {
    const previous = { version: "1.0", versionUpdatedAtUnixSeconds: 123, versionUpdatedAtSource: "observed" };
    expect(mergeVersionUpdatedAt(previous, "1.0", null, now)).toEqual({ time: 123, source: "observed" });
  });

  it("official candidate always wins (and backfills a null baseline)", () => {
    const previous = { version: "1.0", versionUpdatedAtUnixSeconds: null, versionUpdatedAtSource: null };
    const candidate = { time: 999, source: "official" };
    expect(mergeVersionUpdatedAt(previous, "1.0", candidate, now)).toEqual(candidate);
    expect(mergeVersionUpdatedAt(null, "1.0", candidate, now)).toEqual(candidate);
  });

  it("serverModified adopted on first sight / version change / previous observed", () => {
    const candidate = { time: 777, source: "serverModified" };
    // no previous
    expect(mergeVersionUpdatedAt(null, "1.0", candidate, now)).toEqual({ time: 777, source: "serverModified" });
    // version changed
    const prevChanged = { version: "1.0", versionUpdatedAtUnixSeconds: 100, versionUpdatedAtSource: "official" };
    expect(mergeVersionUpdatedAt(prevChanged, "2.0", candidate, now)).toEqual({ time: 777, source: "serverModified" });
    // previous was observed
    const prevObserved = { version: "1.0", versionUpdatedAtUnixSeconds: 100, versionUpdatedAtSource: "observed" };
    expect(mergeVersionUpdatedAt(prevObserved, "1.0", candidate, now)).toEqual({ time: 777, source: "serverModified" });
  });

  it("serverModified does not churn an existing official timestamp on same version", () => {
    const previous = { version: "1.0", versionUpdatedAtUnixSeconds: 100, versionUpdatedAtSource: "official" };
    const candidate = { time: 777, source: "serverModified" };
    expect(mergeVersionUpdatedAt(previous, "1.0", candidate, now)).toEqual({ time: 100, source: "official" });
  });
});
