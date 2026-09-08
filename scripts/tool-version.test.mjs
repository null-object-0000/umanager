import { describe, expect, it } from "vitest";
import { npmDistTagChannels, resolveNpmDistTagVersion } from "./tool-version.mjs";

const DOC = {
  name: "@deepseek-ai/dsh",
  "dist-tags": {
    latest: "0.1.1-rc.2",
    alpha: "0.1.2-alpha.5",
    next: "0.1.1-rc.2",
  },
};

describe("resolveNpmDistTagVersion", () => {
  it("defaults to the latest tag", () => {
    expect(resolveNpmDistTagVersion(DOC)).toBe("0.1.1-rc.2");
    expect(resolveNpmDistTagVersion(DOC, undefined)).toBe("0.1.1-rc.2");
  });

  it("resolves an explicit dist-tag such as alpha", () => {
    expect(resolveNpmDistTagVersion(DOC, "alpha")).toBe("0.1.2-alpha.5");
    expect(resolveNpmDistTagVersion(DOC, "next")).toBe("0.1.1-rc.2");
  });

  it("rejects a missing dist-tag loudly instead of falling back", () => {
    expect(() => resolveNpmDistTagVersion(DOC, "alpa")).toThrow(/未发布版本/);
    expect(() => resolveNpmDistTagVersion(DOC, "")).not.toThrow();
  });

  it("rejects non-object documents and missing dist-tags blocks", () => {
    expect(() => resolveNpmDistTagVersion(null)).toThrow(/不是有效对象/);
    expect(() => resolveNpmDistTagVersion("nope")).toThrow(/不是有效对象/);
    expect(() => resolveNpmDistTagVersion({})).toThrow(/未返回 dist-tags/);
  });

  it("rejects malformed tag names", () => {
    expect(() => resolveNpmDistTagVersion(DOC, "bad tag!")).toThrow(/非法 npm dist-tag/);
    expect(() => resolveNpmDistTagVersion(DOC, "../etc")).toThrow(/非法 npm dist-tag/);
  });
});

describe("npmDistTagChannels", () => {
  it("returns every dist-tag as a sorted tag -> version map", () => {
    expect(npmDistTagChannels(DOC)).toEqual({
      alpha: "0.1.2-alpha.5",
      latest: "0.1.1-rc.2",
      next: "0.1.1-rc.2",
    });
  });

  it("skips malformed tags and versions instead of echoing them", () => {
    const doc = {
      "dist-tags": {
        latest: "0.1.0",
        "bad tag!": "0.2.0",
        "next": "",
        "": "0.3.0",
      },
    };
    expect(npmDistTagChannels(doc)).toEqual({ latest: "0.1.0" });
  });

  it("returns null when there are no usable dist-tags", () => {
    expect(npmDistTagChannels(null)).toBeNull();
    expect(npmDistTagChannels({})).toBeNull();
    expect(npmDistTagChannels({ "dist-tags": {} })).toBeNull();
    expect(npmDistTagChannels({ "dist-tags": { "bad tag!": "0.1.0" } })).toBeNull();
    expect(npmDistTagChannels("nope")).toBeNull();
  });
});