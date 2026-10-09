// Guard for the variant metadata in feed-sources.json (CI-only config).
//
// `variantGroup` / `variantLabel` / `variantDefault` are copied verbatim into the
// signed catalog and drive the store's "one card + variant switch" presentation.
// A typo there is invisible at build time (every field is optional), so it is
// asserted here: a broken group would silently collapse two products into one
// card with a missing or duplicated label.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { describe, expect, it } from "vitest";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const config = JSON.parse(readFileSync(resolve(REPO_ROOT, "feed-sources.json"), "utf8"));

const groups = new Map();
for (const entry of config.applications) {
  const group = typeof entry.variantGroup === "string" ? entry.variantGroup.trim() : "";
  if (!group) continue;
  const members = groups.get(group);
  if (members) members.push(entry);
  else groups.set(group, [entry]);
}

describe("feed-sources.json variant groups", () => {
  it("declares at least one variant group", () => {
    expect(groups.size).toBeGreaterThan(0);
  });

  it("keeps every applicationId unique", () => {
    const ids = config.applications.map((entry) => entry.applicationId);
    expect(new Set(ids).size).toBe(ids.length);
  });

  // 这两个产品是「国内版 / 国际版」需求的来源；默认变体必须是国内版（面向国内
  // 用户，且国内版包名以 `-cn` 结尾）。
  for (const group of ["qoder", "trae"]) {
    describe(`product ${group}`, () => {
      it("offers exactly the 国内版 / 国际版 pair", () => {
        expect(groups.has(group)).toBe(true);
        expect(groups.get(group).map((member) => member.variantLabel).sort()).toEqual(["国内版", "国际版"].sort());
      });

      it("defaults to the China-market build", () => {
        const fallback = groups.get(group).find((member) => member.variantDefault === true);
        expect(fallback).toBeDefined();
        expect(fallback.packageName.endsWith("-cn")).toBe(true);
      });
    });
  }

  // Wine's two editions are *mutually exclusive* (both `Provides: wine` and
  // `Conflicts: wine`), so the store must offer an uninstall-then-install switch
  // instead of a plain "install the other one" (see DESIGN-app-variants.md).
  describe("product wine", () => {
    it("offers the 稳定版 / 开发版 pair, defaulting to stable", () => {
      expect(groups.has("wine")).toBe(true);
      const members = groups.get("wine");
      expect(members.map((member) => member.variantLabel).sort()).toEqual(["开发版", "稳定版"].sort());
      expect(members.find((member) => member.variantDefault === true).packageName).toBe("winehq-stable");
    });

    it("declares every member exclusive", () => {
      expect(groups.get("wine").every((member) => member.variantExclusive === true)).toBe(true);
    });
  });

  for (const [group, members] of groups) {
    describe(`group ${group}`, () => {
      it("has at least two members (a single member is not a switch)", () => {
        expect(members.length).toBeGreaterThan(1);
      });

      it("gives every member a distinct non-empty label", () => {
        const labels = members.map((member) => member.variantLabel);
        for (const label of labels) {
          expect(typeof label).toBe("string");
          expect(label.trim().length).toBeGreaterThan(0);
        }
        expect(new Set(labels).size).toBe(members.length);
      });

      it("declares exactly one default variant", () => {
        expect(members.filter((member) => member.variantDefault === true).length).toBe(1);
      });

      it("uses one package name per variant (install state is detected per package)", () => {
        const packages = members.map((member) => member.packageName);
        expect(new Set(packages).size).toBe(members.length);
      });

      it("shares one displayName so the card shows the product, not the market", () => {
        expect(new Set(members.map((member) => member.displayName)).size).toBe(1);
      });
    });
  }
});
