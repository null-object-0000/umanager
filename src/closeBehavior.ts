import type { BatchSnapshot, DownloadJob } from "./types";
import { batchActive } from "./batchTasks";

/**
 * 「关闭窗口」的实际语义由后端决定：托盘图标可见时收进托盘、进程常驻，
 * 否则就是真正的退出。退出前必须让用户看到还没结束的工作，否则下载与
 * 批量更新会随着进程一起消失。
 */
export type CloseBehavior = "tray" | "quit";

export interface CloseSummary {
  activeDownloads: number;
  activeBatches: number;
}

export function summarizePendingWork(snapshot: BatchSnapshot, jobs: DownloadJob[]): CloseSummary {
  return {
    activeDownloads: jobs.filter((job) => job.status === "queued" || job.status === "downloading" || job.status === "verifying").length,
    activeBatches: snapshot.batches.filter((batch) => batchActive(batch.status)).length,
  };
}

export function hasPendingWork(summary: CloseSummary): boolean {
  return summary.activeDownloads > 0 || summary.activeBatches > 0;
}

/** 关闭前的提示文案；没有未完成工作时提示「只是关掉」。 */
export function closePrompt(summary: CloseSummary): { title: string; body: string } {
  if (summary.activeBatches > 0) {
    return {
      title: "批量更新还没结束",
      body: `还有 ${summary.activeBatches} 个批量任务在进行中（下载、复核或安装）。现在关闭会中断它们，任务中心也会一起关掉。`,
    };
  }
  if (summary.activeDownloads > 0) {
    return {
      title: "还有下载没完成",
      body: `下载队列里还有 ${summary.activeDownloads} 个任务。下载由 UManager 在后台完成，关掉窗口就会停止。`,
    };
  }
  return { title: "退出 UManager？", body: "本机没有正在进行的下载或更新，退出不会丢失任何任务。" };
}
