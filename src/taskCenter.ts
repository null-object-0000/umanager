import { batchActive } from "./batchTasks";
import { isActiveStatus } from "./downloadQueue";
import type { BatchStatus, BatchTask, DownloadJob } from "./types";

export type TaskFilter = "all" | "active" | "attention" | "history";
export const taskFilters = [["all", "全部"], ["attention", "待处理"], ["active", "进行中"], ["history", "历史记录"]] as const;
export function batchNeedsAttention(status: BatchStatus): boolean {
  return ["awaitingReview", "awaitingConfirmation", "paused", "interrupted"].includes(status);
}
export function matchesBatchFilter(batch: BatchTask, filter: TaskFilter): boolean {
  return filter === "all" || (filter === "attention" ? batchNeedsAttention(batch.status)
    : filter === "active" ? batchActive(batch.status) && !batchNeedsAttention(batch.status) : !batchActive(batch.status));
}
export function matchesDownloadFilter(job: DownloadJob, filter: TaskFilter): boolean {
  return filter === "all" || (filter === "attention" ? job.status === "ready" || job.status === "error"
    : filter === "active" ? isActiveStatus(job.status) : job.status === "canceled" || job.status === "error");
}
// A batch owns its downloads until explicitly ended. Ending it releases any
// retained packages so they can be installed independently.
export function standaloneDownloads(jobs: DownloadJob[], batches: BatchTask[]): DownloadJob[] {
  const owned = new Set(batches.filter((batch) => batch.status !== "canceled").flatMap((batch) => batch.items.map((item) => item.downloadJobId)));
  return jobs.filter((job) => !owned.has(job.jobId));
}
