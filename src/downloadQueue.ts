/// 下载队列的展示逻辑（纯函数，便于单测覆盖）。队列本身与请求由 `api.ts` / Rust 侧的
/// `download_queue.rs` 负责，这里只决定「怎么排序、显示什么文案、卡片画什么」。

import type { DownloadJob, DownloadJobStatus, DownloadProgress } from "./types";

/// 排序权重：进行中 → 排队中 → 待安装 → 已结束（失败 / 已取消）。
const STATUS_WEIGHT: Record<DownloadJobStatus, number> = {
  downloading: 0,
  verifying: 0,
  queued: 1,
  ready: 2,
  error: 3,
  canceled: 3,
};

export function isActiveStatus(status: DownloadJobStatus): boolean {
  return status === "queued" || status === "downloading" || status === "verifying";
}

/// 侧边栏徽标：还在队列里（排队 + 下载 + 校验）的任务数。待安装不算「进行中」。
export function activeJobCount(jobs: DownloadJob[]): number {
  return jobs.filter((job) => isActiveStatus(job.status)).length;
}

/// 「下载」页排序：先按状态分组，组内按入队时间（先入队的在前，与队列执行顺序一致）。
export function sortJobs(jobs: DownloadJob[]): DownloadJob[] {
  return [...jobs].sort((left, right) => {
    const weight = STATUS_WEIGHT[left.status] - STATUS_WEIGHT[right.status];
    if (weight !== 0) return weight;
    if (left.enqueuedAtUnixSeconds !== right.enqueuedAtUnixSeconds) {
      return left.enqueuedAtUnixSeconds - right.enqueuedAtUnixSeconds;
    }
    return left.jobId.localeCompare(right.jobId);
  });
}

/// 找某个软件包当前的任务：优先进行中 / 待安装，其次最近结束的那条（卡片据此显示
/// 「下载中」圆环、「安装」按钮或失败状态）。
export function findJob(jobs: DownloadJob[], packageName: string): DownloadJob | undefined {
  const candidates = jobs.filter((job) => job.packageName === packageName);
  const preferred = candidates.filter((job) => isActiveStatus(job.status) || job.status === "ready");
  const pool = preferred.length > 0 ? preferred : candidates;
  return sortJobs(pool)[0];
}

/// 进行中任务的进度载荷（卡片圆环 / 抽屉 hero 用），其余状态返回 null。
export function jobProgress(job: DownloadJob | undefined): DownloadProgress | null {
  if (!job) return null;
  return job.status === "downloading" || job.status === "verifying" ? job.progress : null;
}

/// 状态文案。`queuedPosition` 是它在队列里的位次（从 1 开始，0 / undefined 表示不显示）。
export function jobStatusLabel(job: DownloadJob, queuedPosition?: number): string {
  switch (job.status) {
    case "queued":
      return queuedPosition && queuedPosition > 0 ? `排队中 · 第 ${queuedPosition} 位` : "排队中";
    case "downloading":
      return "下载中";
    case "verifying":
      return "校验中";
    case "ready":
      return "待安装";
    case "error":
      return "下载失败";
    case "canceled":
      return "已取消";
  }
}

/// 排队位次（只统计排队中的任务，先入队的排前面）。
export function queuedPosition(jobs: DownloadJob[], job: DownloadJob): number {
  if (job.status !== "queued") return 0;
  const queued = sortJobs(jobs).filter((entry) => entry.status === "queued");
  return queued.findIndex((entry) => entry.jobId === job.jobId) + 1;
}

/// 队列页主按钮文案：待安装的任务在「软件」列表里可能是安装，也可能是更新。
export function installActionLabel(installed: boolean): string {
  return installed ? "更新" : "安装";
}

export function jobPercent(progress: DownloadProgress | null): number {
  if (!progress || progress.totalBytes <= 0) return 0;
  return Math.min(100, Math.round((progress.transferredBytes / progress.totalBytes) * 100));
}
