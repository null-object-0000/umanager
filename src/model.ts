import type { ManagedPackage } from "./types";

/// 商店列表的排序方式。默认 `updatedDesc`：按「当前版本的发布时间」
/// （`versionUpdatedAtUnixSeconds`）从新到旧 —— 也就是每个软件的最新版本是
/// 什么时候发布的。
export type SortMode = "updatedDesc" | "updatedAsc" | "nameAsc";

export const sortModeLabels: ReadonlyArray<{ value: SortMode; label: string }> = [
  { value: "updatedDesc", label: "最近更新" },
  { value: "updatedAsc", label: "最早更新" },
  { value: "nameAsc", label: "名称" },
];

/// 排序商店列表：主键是条目自身的「版本发布时间」，缺失该时间的条目恒定排在
/// 最后（无论升序还是降序 —— 没有时间既不算最新也不算最早），同键时用名称做
/// 稳定兜底。纯函数，便于单测覆盖。
export function sortSoftwareItems<T>(
  items: T[],
  mode: SortMode,
  updatedAt: (item: T) => number | null | undefined,
  displayName: (item: T) => string,
): T[] {
  const byName = (left: T, right: T) => displayName(left).localeCompare(displayName(right), "zh-CN");
  const sorted = [...items];
  if (mode === "nameAsc") return sorted.sort(byName);
  const descending = mode === "updatedDesc";
  return sorted.sort((left, right) => {
    const leftTime = updatedAt(left);
    const rightTime = updatedAt(right);
    const leftKnown = leftTime != null;
    const rightKnown = rightTime != null;
    if (!leftKnown || !rightKnown) {
      if (leftKnown === rightKnown) return byName(left, right);
      return leftKnown ? -1 : 1;
    }
    if (leftTime !== rightTime) return descending ? rightTime - leftTime : leftTime - rightTime;
    return byName(left, right);
  });
}

/// 从持久化的排序偏好里恢复排序方式；任何非法值都回落到默认（最近更新）。
export function readSortMode(stored: string | null | undefined): SortMode {
  return sortModeLabels.find((entry) => entry.value === stored)?.value ?? "updatedDesc";
}

export function summarizePackages(packages: ManagedPackage[]) {
  return {
    total: packages.length,
    updates: packages.filter((item) => item.updateState === "updateAvailable").length,
    repositories: packages.filter((item) => item.sourceKind === "officialRepository").length,
    pendingRepositoryChecks: packages.filter((item) => item.sourceKind === "officialRepository" && item.updateState === "unknown").length,
  };
}
