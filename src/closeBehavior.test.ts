import { describe, expect, it } from "vitest";
import { closePrompt, hasPendingWork, summarizePendingWork } from "./closeBehavior";
import type { BatchSnapshot, BatchStatus, DownloadJob, DownloadJobStatus } from "./types";

function job(status: DownloadJobStatus): DownloadJob {
  return { id: `job-${status}-${Math.random()}`, applicationId: "app", version: "1.0", status } as unknown as DownloadJob;
}

function snapshot(statuses: BatchStatus[]): BatchSnapshot {
  return {
    revision: 1,
    batches: statuses.map((status, index) => ({ id: `b${index}`, status, items: [] } as unknown as BatchSnapshot["batches"][number])),
  };
}

describe("summarizePendingWork", () => {
  it("只统计在排队/下载/校验中的下载与进行中的批量任务", () => {
    const summary = summarizePendingWork(
      snapshot(["downloading", "awaitingConfirmation", "completed", "paused", "canceled"]),
      [job("queued"), job("downloading"), job("verifying"), job("ready" as DownloadJobStatus), job("error" as DownloadJobStatus)],
    );
    expect(summary).toEqual({ activeDownloads: 3, activeBatches: 2 });
  });

  it("没有任务时不算待处理工作", () => {
    const summary = summarizePendingWork(snapshot(["completed"]), []);
    expect(hasPendingWork(summary)).toBe(false);
  });
});

describe("closePrompt", () => {
  it("优先提示批量任务，其次下载，最后才是一般退出", () => {
    expect(closePrompt({ activeDownloads: 2, activeBatches: 1 }).title).toBe("批量更新还没结束");
    expect(closePrompt({ activeDownloads: 2, activeBatches: 0 }).title).toBe("还有下载没完成");
    expect(closePrompt({ activeDownloads: 0, activeBatches: 0 }).title).toBe("退出 UManager？");
  });
});
