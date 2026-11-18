import type { BatchSnapshot, DownloadJob } from "./types";
import { batchActive } from "./batchTasks";

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

/** 根据任务状态生成退出提示。 */
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
