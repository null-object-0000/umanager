import { describe, expect, it } from "vitest";
import {
  conflictingVariant,
  foldVariantCandidates,
  foldVariantGroup,
  groupVariants,
  isExclusiveGroup,
  isSwitchableGroup,
  orderVariants,
  pickVariant,
  readVariantSelection,
  variantLabelOf,
  variantSelectionKey,
} from "./variants";
import type { CatalogApplication } from "./types";

function app(overrides: Partial<CatalogApplication> & { applicationId: string }): CatalogApplication {
  return {
    packageName: overrides.applicationId,
    displayName: "Qoder",
    vendor: "阿里巴巴",
    architecture: "amd64",
    homepage: null,
    icon: null,
    accentColor: null,
    removable: true,
    source: { kind: "stableDownloadEndpoint" },
    ...overrides,
  };
}

const cn = app({ applicationId: "qoder-cn", packageName: "qoder-cn", variantGroup: "qoder", variantLabel: "国内版", variantDefault: true });
const intl = app({ applicationId: "qoder", packageName: "qoder", variantGroup: "qoder", variantLabel: "国际版" });
const solo = app({ applicationId: "cursor", packageName: "cursor", displayName: "Cursor", variantLabel: null });

describe("variant grouping", () => {
  it("groups entries that share a variantGroup and ignores ungrouped ones", () => {
    const groups = groupVariants([intl, solo, cn]);
    expect([...groups.keys()]).toEqual(["qoder"]);
    expect(groups.get("qoder")?.map((item) => item.applicationId)).toEqual(["qoder-cn", "qoder"]);
  });

  it("puts the declared default variant first regardless of feed order", () => {
    expect(orderVariants([intl, cn]).map((item) => item.applicationId)).toEqual(["qoder-cn", "qoder"]);
    // Without a default the order is by applicationId, so it stays stable.
    expect(orderVariants([intl]).map((item) => item.applicationId)).toEqual(["qoder"]);
  });

  it("does not mutate the input array", () => {
    const input = [intl, cn];
    orderVariants(input);
    expect(input.map((item) => item.applicationId)).toEqual(["qoder", "qoder-cn"]);
  });

  it("treats a single-member group as not switchable", () => {
    expect(isSwitchableGroup([cn, intl])).toBe(true);
    expect(isSwitchableGroup([cn])).toBe(false);
  });

  it("falls back to the display name when no variant label is declared", () => {
    expect(variantLabelOf(cn)).toBe("国内版");
    expect(variantLabelOf(solo)).toBe("Cursor");
    expect(variantLabelOf(app({ applicationId: "x", variantLabel: "   " }))).toBe("Qoder");
  });
});

describe("pickVariant", () => {
  const group = [cn, intl];

  it("keeps the stored choice when that variant is installed", () => {
    expect(pickVariant(group, { installedApplicationIds: ["qoder-cn", "qoder"], selectedApplicationId: "qoder" })?.applicationId)
      .toBe("qoder");
  });

  it("never hides an installed variant behind a stored choice", () => {
    // 用户上次看的是国际版，但本机装的是国内版：卡片必须显示已安装的国内版。
    expect(pickVariant(group, { installedApplicationIds: ["qoder-cn"], selectedApplicationId: "qoder" })?.applicationId)
      .toBe("qoder-cn");
  });

  it("prefers the stored choice when nothing is installed", () => {
    expect(pickVariant(group, { selectedApplicationId: "qoder" })?.applicationId).toBe("qoder");
  });

  it("falls back to the declared default", () => {
    expect(pickVariant(group, {})?.applicationId).toBe("qoder-cn");
    expect(pickVariant([intl], {})?.applicationId).toBe("qoder");
  });

  it("returns null for an empty group", () => {
    expect(pickVariant([], {})).toBeNull();
  });
});

describe("foldVariantGroup", () => {
  const cnItem = { id: "cn-item", installed: false };
  const intlItem = { id: "intl-item", installed: false };

  it("folds members into one primary plus the ordered variant list", () => {
    const folded = foldVariantGroup([
      { item: intlItem, application: intl, installed: false },
      { item: cnItem, application: cn, installed: false },
    ]);
    expect(folded?.primary).toBe(cnItem);
    expect(folded?.primaryApplication.applicationId).toBe("qoder-cn");
    expect(folded?.variants.map((variant) => variant.applicationId)).toEqual(["qoder-cn", "qoder"]);
  });

  it("picks the installed member even when another variant was selected", () => {
    const folded = foldVariantGroup([
      { item: cnItem, application: cn, installed: true },
      { item: intlItem, application: intl, installed: false },
    ], "qoder");
    expect(folded?.primary).toBe(cnItem);
  });

  it("returns null for an empty group", () => {
    expect(foldVariantGroup([])).toBeNull();
  });
});

describe("foldVariantCandidates", () => {
  const plainItem = { id: "cursor-item" };
  const cnItem = { id: "cn-item" };
  const intlItem = { id: "intl-item" };

  const candidates = [
    { item: plainItem, application: solo, installed: false },
    { item: cnItem, application: cn, installed: false },
    { item: intlItem, application: intl, installed: false },
  ];

  it("keeps ungrouped entries in place and folds a group into its first member", () => {
    const { items, folded } = foldVariantCandidates(candidates, { selectedApplicationIdOf: () => null });
    // 组里第一个成员（国内版）当容器，国际版被并入；无关条目顺序不变。
    expect(items).toEqual([plainItem, cnItem]);
    expect(folded.size).toBe(1);
    const group = folded.get(cnItem);
    expect(group?.group).toBe("qoder");
    expect(group?.primary).toBe(cnItem);
    expect(group?.variants.map((variant) => variant.applicationId)).toEqual(["qoder-cn", "qoder"]);
  });

  it("drops a group with a single member (no switch to offer)", () => {
    const onlyCn = candidates.filter((candidate) => candidate !== candidates[1]);
    const { items, folded } = foldVariantCandidates(onlyCn, { selectedApplicationIdOf: () => null });
    expect(items).toEqual([plainItem, intlItem]);
    expect(folded.size).toBe(0);
  });

  it("honours the stored selection when nothing is installed", () => {
    const first = { item: { id: "first" }, application: cn, installed: false };
    const second = { item: { id: "second" }, application: intl, installed: false };
    const { folded } = foldVariantCandidates([first, second], { selectedApplicationIdOf: (group) => group === "qoder" ? "qoder" : null });
    expect(folded.get(first.item)?.primary).toBe(second.item);
  });

  it("can fold a group whose members are not adjacent", () => {
    const first = { item: { id: "a" }, application: cn, installed: false };
    const second = { item: { id: "b" }, application: intl, installed: true };
    const { items, folded } = foldVariantCandidates([first, second], { selectedApplicationIdOf: () => null });
    expect(items).toEqual([first.item]);
    expect(folded.get(first.item)?.primary).toBe(second.item);
  });
});

describe("variant selection persistence", () => {
  it("namespaces the key by group", () => {
    expect(variantSelectionKey("qoder")).toBe("umanager.app-variant.qoder");
  });

  it("drops a stored selection that is not part of the group", () => {
    expect(readVariantSelection("qoder", [cn])).toBeNull();
    expect(readVariantSelection("qoder", [cn, intl])).toBe("qoder");
    expect(readVariantSelection(null, [cn, intl])).toBeNull();
  });
});

describe("mutually exclusive variant groups (Wine 稳定版 / 开发版)", () => {
  const devel = app({ applicationId: "wine", packageName: "winehq-devel", displayName: "Wine", variantGroup: "wine", variantLabel: "开发版", variantExclusive: true });
  const stable = app({ applicationId: "wine-stable", packageName: "winehq-stable", displayName: "Wine", variantGroup: "wine", variantLabel: "稳定版", variantDefault: true, variantExclusive: true });
  const group = [stable, devel];

  it("treats a group as exclusive only when every member declares it", () => {
    expect(isExclusiveGroup(group)).toBe(true);
    expect(isExclusiveGroup([stable, { ...devel, variantExclusive: undefined }])).toBe(false);
    expect(isExclusiveGroup([stable])).toBe(false);
    // 国内版 / 国际版可以共存：没有互斥标记，不提示卸载。
    expect(isExclusiveGroup([cn, intl])).toBe(false);
  });

  it("finds the installed sibling that blocks installing the target", () => {
    expect(conflictingVariant(group, "wine-stable", ["wine"])?.applicationId).toBe("wine");
    expect(conflictingVariant(group, "wine", ["wine-stable"])?.applicationId).toBe("wine-stable");
  });

  it("reports no conflict when the target is already installed or nothing is installed", () => {
    expect(conflictingVariant(group, "wine", ["wine"])).toBeNull();
    expect(conflictingVariant(group, "wine-stable", [])).toBeNull();
    expect(conflictingVariant(group, "wine-stable", ["cursor"])).toBeNull();
    // 可共存的组永远不会因为兄弟已安装而要求先卸载。
    expect(conflictingVariant([cn, intl], "qoder", ["qoder-cn"])).toBeNull();
  });
});
