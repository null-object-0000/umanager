import type { BatchItem, BatchStatus, BatchTask, CatalogApplication, ManagedPackage, ScanResult } from "./types";

export interface BatchCandidate { applicationId: string; package: ManagedPackage }
export const batchStatusLabels: Record<BatchStatus, string> = {
  preparing: "准备下载", downloading: "下载与校验中", awaitingReview: "等待授权复核", reviewing: "管理员复核中",
  awaitingConfirmation: "等待再次确认", installing: "串行安装中", completed: "全部完成", paused: "需要处理", interrupted: "上次运行中断", canceled: "已结束",
};
export const batchItemLabels: Record<BatchItem["status"], string> = {
  downloading: "等待下载校验", ready: "待复核", reviewed: "复核通过", installing: "正在安装", succeeded: "更新成功", failed: "失败", blocked: "未执行，需重新检查", unknown: "安装结果待核对", canceled: "已结束",
};
export function batchActive(status: BatchStatus): boolean {
  return ["preparing", "downloading", "awaitingReview", "reviewing", "awaitingConfirmation", "installing"].includes(status);
}
export function batchCandidates(scan: ScanResult, catalog: CatalogApplication[], ids?: string[]): BatchCandidate[] {
  const allowed = ids ? new Set(ids) : null;
  // Use raw installed packages, preserving independently installed variants.
  return scan.packages.flatMap((pkg) => {
    const app = catalog.find((a) => a.packageName === pkg.packageName);
    return app && app.applicationId !== "umanager" && app.source.kind !== "browserImport"
      && pkg.updateState === "updateAvailable" && pkg.candidateVersion && (!allowed || allowed.has(app.applicationId))
      ? [{ applicationId: app.applicationId, package: pkg }] : [];
  });
}
export function unfinishedIds(batch: BatchTask): string[] {
  return batch.items.filter((i) => i.status !== "succeeded").map((i) => i.applicationId);
}
export function reviewExpiresAt(batch: BatchTask): number | null {
  const expiry = batch.items.filter((i) => i.status === "reviewed").map((i) => i.expiresAtUnixSeconds);
  return expiry.length && expiry.every((e): e is number => e !== null) ? Math.min(...expiry as number[]) : null;
}
export function canConfirmBatch(batch: BatchTask, nowSeconds: number): boolean {
  const expiry = reviewExpiresAt(batch);
  return batch.status === "awaitingConfirmation" && !!batch.reviewId && expiry !== null && nowSeconds <= expiry;
}
