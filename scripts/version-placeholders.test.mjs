import { describe, expect, it } from "vitest";
import { debianUpstreamVersion, resolveVersionPlaceholders } from "./version-placeholders.mjs";

describe("debianUpstreamVersion", () => {
  it("drops WineHQ's vendor suffix and Debian revision", () => {
    expect(debianUpstreamVersion("11.19~resolute-1")).toBe("11.19");
    expect(debianUpstreamVersion("11.19~noble-1")).toBe("11.19");
  });

  it("trims trailing zero components so Wine's padded stable versions match their tags", () => {
    expect(debianUpstreamVersion("11.0.0.0~resolute-1")).toBe("11.0");
    expect(debianUpstreamVersion("9.0.1.0~resolute-1")).toBe("9.0.1");
    expect(debianUpstreamVersion("10.0.0.0~oracular-1")).toBe("10.0");
  });

  it("keeps versions that are already upstream-shaped", () => {
    expect(debianUpstreamVersion("1.135.0-1787669172")).toBe("1.135.0");
    expect(debianUpstreamVersion("4.1.0.13")).toBe("4.1.0.13");
    expect(debianUpstreamVersion("141.0.7390.54-1")).toBe("141.0.7390.54");
  });

  it("drops a Debian epoch and returns '' for unusable input", () => {
    expect(debianUpstreamVersion("1:5.1.9")).toBe("5.1.9");
    expect(debianUpstreamVersion("1:5.1.9-2")).toBe("5.1.9");
    expect(debianUpstreamVersion("11.0~rc5~resolute-1")).toBe("11.0");
    expect(debianUpstreamVersion(null)).toBe("");
    expect(debianUpstreamVersion(undefined)).toBe("");
  });
});

describe("resolveVersionPlaceholders", () => {
  it("keeps the existing {version} / {major} / {minor} / {patch} semantics", () => {
    expect(resolveVersionPlaceholders("v{major}_{minor}", "1.135.0-1787669172")).toBe("v1_135");
    expect(resolveVersionPlaceholders("updates?version={version}", "4.1.0.13")).toBe("updates?version=4.1.0.13");
    expect(resolveVersionPlaceholders("{major}.{minor}.{patch}", "1.135.0-1787669172")).toBe("1.135.0-1787669172");
  });

  it("resolves {upstream} for Wine's release tags", () => {
    expect(resolveVersionPlaceholders("wine-{upstream}/ANNOUNCE.md", "11.19~resolute-1")).toBe("wine-11.19/ANNOUNCE.md");
    expect(resolveVersionPlaceholders("wine-{upstream}/ANNOUNCE.md", "11.0.0.0~resolute-1")).toBe("wine-11.0/ANNOUNCE.md");
    expect(resolveVersionPlaceholders("wine-{upstream}/ANNOUNCE.md", "9.0.1.0~resolute-1")).toBe("wine-9.0.1/ANNOUNCE.md");
  });

  it("passes non-string templates through and encodes values", () => {
    expect(resolveVersionPlaceholders(undefined, "1.0")).toBeUndefined();
    expect(resolveVersionPlaceholders("x{version}", "1.0+2")).toBe("x1.0%2B2");
  });
});
