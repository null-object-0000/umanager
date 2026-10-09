import type { CatalogApplication } from "./types";

/// 同一款软件的不同发行变体（把同一产品分市场发布的两个独立安装包，例如
/// Qoder 国内版 / 国际版）在签名 feed 里共享一个 `variantGroup`。商店把它们折叠
/// 成一张卡片，详情页提供与 dsh「版本线」同款的分段切换。
///
/// 关键取舍：变体折叠是**展示层**的概念 —— 每个变体仍然是完整的目录条目（自己
/// 的 applicationId / 包名 / 下载域名 / SHA-256），安装、更新、卸载走的都是原有
/// 的单条目链路。这样特权 helper 的授权链（内置公钥 + 计划内已签名 catalogJson）
/// 完全不需要放宽，老版本 App 也只会看到两个普通条目。
///
/// 本模块只放纯函数，便于单测覆盖。

/// 组内排序：默认变体（`variantDefault`）恒定排在最前，其余按 applicationId
/// 稳定排序，保证切换控件的顺序在不同 feed 顺序下也一致。不修改入参。
export function orderVariants(variants: CatalogApplication[]): CatalogApplication[] {
  return [...variants].sort((left, right) => {
    const leftDefault = left.variantDefault === true ? 0 : 1;
    const rightDefault = right.variantDefault === true ? 0 : 1;
    if (leftDefault !== rightDefault) return leftDefault - rightDefault;
    return left.applicationId.localeCompare(right.applicationId, "en");
  });
}

/// 切换控件上的短标签：feed 声明的 `variantLabel`，缺失时回落到软件名。
export function variantLabelOf(application: CatalogApplication): string {
  const label = application.variantLabel?.trim();
  return label ? label : application.displayName;
}

/// 按 `variantGroup` 归组；没有 `variantGroup` 的条目不属于任何变体组。
export function groupVariants(
  applications: CatalogApplication[],
): Map<string, CatalogApplication[]> {
  const groups = new Map<string, CatalogApplication[]>();
  for (const application of applications) {
    const group = application.variantGroup?.trim();
    if (!group) continue;
    const members = groups.get(group);
    if (members) members.push(application);
    else groups.set(group, [application]);
  }
  for (const [group, members] of groups) groups.set(group, orderVariants(members));
  return groups;
}

/// 只有一个成员的分组不构成「切换」，按普通条目处理。
export function isSwitchableGroup(variants: CatalogApplication[]): boolean {
  return variants.length > 1;
}

/// 组内变体是否**互斥**（不能同时安装）。要求组内每个成员都声明
/// `variantExclusive`：feed 里漏标一个成员就按可共存处理（保守），不会误报
/// 「需要先卸载」。
export function isExclusiveGroup(variants: CatalogApplication[]): boolean {
  return variants.length > 1 && variants.every((variant) => variant.variantExclusive === true);
}

/// 切到 `targetApplicationId` 时会与哪个**已安装的**兄弟变体冲突（必须先卸载
/// 它）。只在互斥组、目标未安装、且有兄弟已安装时返回；国内版 / 国际版这种可
/// 共存的组恒为 null，因此不会出现多余的卸载提示。
export function conflictingVariant(
  variants: CatalogApplication[],
  targetApplicationId: string,
  installedApplicationIds: Iterable<string>,
): CatalogApplication | null {
  if (!isExclusiveGroup(variants)) return null;
  const installed = new Set(installedApplicationIds);
  if (installed.has(targetApplicationId)) return null;
  return (
    variants.find(
      (variant) => variant.applicationId !== targetApplicationId && installed.has(variant.applicationId),
    ) ?? null
  );
}

/// 卡片与详情页默认展示哪个变体。优先级：
///   1. 用户上次选择且该变体**已安装** —— 两个变体都装着时，尊重用户的选择；
///   2. 任一已安装的变体 —— 卡片必须反映本机真实状态，不能因为用户切过一次就
///      把已安装的国内版显示成「未安装」；
///   3. 用户上次选择的变体 —— 什么都没装时，沿用上次浏览的那个；
///   4. feed 声明的默认变体（`variantDefault`），没有则组内第一个。
export function pickVariant(
  variants: CatalogApplication[],
  options: { installedApplicationIds?: Iterable<string>; selectedApplicationId?: string | null } = {},
): CatalogApplication | null {
  if (variants.length === 0) return null;
  const ordered = orderVariants(variants);
  const installed = new Set(options.installedApplicationIds ?? []);
  const selected = options.selectedApplicationId
    ? ordered.find((variant) => variant.applicationId === options.selectedApplicationId) ?? null
    : null;
  if (selected && installed.has(selected.applicationId)) return selected;
  const installedVariant = ordered.find((variant) => installed.has(variant.applicationId));
  if (installedVariant) return installedVariant;
  if (selected) return selected;
  return ordered[0];
}

/// 用户变体选择的持久化键（按变体组隔离）。
export function variantSelectionKey(groupId: string): string {
  return `umanager.app-variant.${groupId}`;
}

/// 读取持久化的变体选择；只在它确实是该组内的合法变体时才采纳。
export function readVariantSelection(
  stored: string | null | undefined,
  variants: CatalogApplication[],
): string | null {
  if (!stored) return null;
  return variants.some((variant) => variant.applicationId === stored) ? stored : null;
}

/// 一个待折叠的条目：它自己（列表项/队列项…）+ 它对应的目录条目 + 是否已安装。
export type VariantFoldEntry<T> = {
  item: T;
  application: CatalogApplication;
  installed: boolean;
};

export type FoldedVariantGroup<T> = {
  /// 「当前展示的变体」对应的原始条目：商店卡片与详情页都用它。
  primary: T;
  primaryApplication: CatalogApplication;
  /// 组内全部变体（默认变体在前），供详情页切换控件渲染。
  variants: CatalogApplication[];
};

/// 把同一变体组的条目折叠成一个主条目 + 变体列表。传 `selected` 时沿用用户上次
/// 的选择（通过 `readVariantSelection` 校验过再传进来）。
export function foldVariantGroup<T>(
  entries: VariantFoldEntry<T>[],
  selectedApplicationId?: string | null,
): FoldedVariantGroup<T> | null {
  if (entries.length === 0) return null;
  const variants = orderVariants(entries.map((entry) => entry.application));
  const installed = entries
    .filter((entry) => entry.installed)
    .map((entry) => entry.application.applicationId);
  const primaryApplication = pickVariant(variants, {
    installedApplicationIds: installed,
    selectedApplicationId,
  });
  if (!primaryApplication) return null;
  const primaryEntry = entries.find(
    (entry) => entry.application.applicationId === primaryApplication.applicationId,
  );
  if (!primaryEntry) return null;
  return { primary: primaryEntry.item, primaryApplication, variants };
}

/// 折叠所需的输入：列表条目 + 它的目录条目（非 .deb 条目为 null）+ 是否已安装。
export type VariantCandidate<T> = {
  item: T;
  application: CatalogApplication | null;
  installed: boolean;
};

export type VariantFold<T> = FoldedVariantGroup<T> & {
  /// 该容器代表的变体组 id。
  group: string;
};

/// 折叠一批条目：不属于变体组的条目原样保留，变体组只保留**第一个成员**作为容器
/// （保持列表原有顺序），组内其它成员并入它。返回容器顺序与容器 → 折叠结果。
export function foldVariantCandidates<T>(
  candidates: VariantCandidate<T>[],
  options: { selectedApplicationIdOf: (group: string) => string | null },
): { items: T[]; folded: Map<T, VariantFold<T>> } {
  const items: T[] = [];
  const groups = new Map<string, { container: T; members: VariantFoldEntry<T>[] }>();
  for (const candidate of candidates) {
    const group = candidate.application?.variantGroup?.trim();
    if (!candidate.application || !group) {
      items.push(candidate.item);
      continue;
    }
    const entry: VariantFoldEntry<T> = {
      item: candidate.item,
      application: candidate.application,
      installed: candidate.installed,
    };
    const existing = groups.get(group);
    if (existing) existing.members.push(entry);
    else {
      items.push(candidate.item);
      groups.set(group, { container: candidate.item, members: [entry] });
    }
  }
  const folded = new Map<T, VariantFold<T>>();
  for (const [group, { container, members }] of groups) {
    const stored = readVariantSelection(
      options.selectedApplicationIdOf(group),
      members.map((member) => member.application),
    );
    const result = foldVariantGroup(members, stored);
    // 只有一个成员的「组」不构成切换，按普通条目处理。
    if (!result || result.variants.length < 2) continue;
    folded.set(container, { ...result, group });
  }
  return { items, folded };
}
