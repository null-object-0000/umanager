import { describe, expect, it } from "vitest";
import { activeJobCount, findJob, installActionLabel, isActiveStatus, jobPercent, jobProgress, jobStatusLabel, queuedPosition, sortJobs } from "./downloadQueue";
import type { DownloadJob, DownloadJobStatus } from "./types";

function job(jobId: string, packageName: string, status: DownloadJobStatus, enqueuedAtUnixSeconds: number, extra: Partial<DownloadJob> = {}): DownloadJob {
  return {
    jobId,
    applicationId: packageName,
    packageName,
    displayName: packageName,
    version: "1.0.0",
    status,
    progress: null,
    error: null,
    result: null,
    enqueuedAtUnixSeconds,
    finishedAtUnixSeconds: null,
    ...extra,
  };
}

const progress = { packageName: "a", phase: "downloading" as const, transferredBytes: 50, totalBytes: 200, bytesPerSecond: 1024 };

describe("isActiveStatus", () => {
  it("counts queued, downloading and verifying as in-flight", () => {
    expect(isActiveStatus("queued")).toBe(true);
    expect(isActiveStatus("downloading")).toBe(true);
    expect(isActiveStatus("verifying")).toBe(true);
    expect(isActiveStatus("ready")).toBe(false);
    expect(isActiveStatus("error")).toBe(false);
    expect(isActiveStatus("canceled")).toBe(false);
  });
});

describe("activeJobCount", () => {
  it("drives the sidebar badge and ignores finished work", () => {
    const jobs = [
      job("1", "a", "queued", 1),
      job("2", "b", "downloading", 2),
      job("3", "c", "verifying", 3),
      job("4", "d", "ready", 4),
      job("5", "e", "error", 5),
      job("6", "f", "canceled", 6),
    ];
    expect(activeJobCount(jobs)).toBe(3);
    expect(activeJobCount([])).toBe(0);
  });
});

describe("sortJobs", () => {
  it("puts in-flight work first, then queued, then ready, then finished", () => {
    const jobs = [
      job("ready", "d", "ready", 1),
      job("error", "e", "error", 2),
      job("queued", "b", "queued", 3),
      job("downloading", "a", "downloading", 4),
      job("verifying", "c", "verifying", 5),
    ];
    expect(sortJobs(jobs).map((entry) => entry.jobId)).toEqual(["downloading", "verifying", "queued", "ready", "error"]);
  });

  it("keeps queue order inside a group", () => {
    const jobs = [job("second", "b", "queued", 20), job("first", "a", "queued", 10)];
    expect(sortJobs(jobs).map((entry) => entry.jobId)).toEqual(["first", "second"]);
  });

  it("does not mutate the input array", () => {
    const jobs = [job("second", "b", "queued", 20), job("first", "a", "queued", 10)];
    sortJobs(jobs);
    expect(jobs.map((entry) => entry.jobId)).toEqual(["second", "first"]);
  });
});

describe("findJob", () => {
  it("prefers an unfinished job over an older finished one", () => {
    const jobs = [job("old", "a", "error", 1), job("new", "a", "queued", 2)];
    expect(findJob(jobs, "a")?.jobId).toBe("new");
  });

  it("prefers a ready job over a failed one", () => {
    const jobs = [job("failed", "a", "error", 1), job("ready", "a", "ready", 2)];
    expect(findJob(jobs, "a")?.jobId).toBe("ready");
  });

  it("falls back to the most recent finished job and returns nothing for unknown packages", () => {
    const jobs = [job("failed", "a", "error", 1), job("canceled", "a", "canceled", 2)];
    expect(findJob(jobs, "a")?.jobId).toBe("failed");
    expect(findJob(jobs, "missing")).toBeUndefined();
  });
});

describe("jobProgress", () => {
  it("only exposes progress while a job is in flight", () => {
    expect(jobProgress(job("1", "a", "downloading", 1, { progress }))).toEqual(progress);
    expect(jobProgress(job("2", "a", "verifying", 1, { progress }))).toEqual(progress);
    expect(jobProgress(job("3", "a", "ready", 1, { progress }))).toBeNull();
    expect(jobProgress(undefined)).toBeNull();
  });
});

describe("jobStatusLabel", () => {
  it("names every status and shows the queue position", () => {
    expect(jobStatusLabel(job("1", "a", "queued", 1), 3)).toBe("排队中 · 第 3 位");
    expect(jobStatusLabel(job("1", "a", "queued", 1))).toBe("排队中");
    expect(jobStatusLabel(job("1", "a", "downloading", 1))).toBe("下载中");
    expect(jobStatusLabel(job("1", "a", "verifying", 1))).toBe("校验中");
    expect(jobStatusLabel(job("1", "a", "ready", 1))).toBe("待安装");
    expect(jobStatusLabel(job("1", "a", "error", 1))).toBe("下载失败");
    expect(jobStatusLabel(job("1", "a", "canceled", 1))).toBe("已取消");
  });
});

describe("queuedPosition", () => {
  it("counts only queued jobs, in queue order", () => {
    const jobs = [job("a", "a", "downloading", 1), job("b", "b", "queued", 2), job("c", "c", "queued", 3)];
    expect(queuedPosition(jobs, jobs[1])).toBe(1);
    expect(queuedPosition(jobs, jobs[2])).toBe(2);
    expect(queuedPosition(jobs, jobs[0])).toBe(0);
  });
});

describe("installActionLabel", () => {
  it("says 更新 for an installed package and 安装 otherwise", () => {
    expect(installActionLabel(true)).toBe("更新");
    expect(installActionLabel(false)).toBe("安装");
  });
});

describe("jobPercent", () => {
  it("clamps and guards against a missing total size", () => {
    expect(jobPercent(progress)).toBe(25);
    expect(jobPercent(null)).toBe(0);
    expect(jobPercent({ ...progress, totalBytes: 0 })).toBe(0);
    expect(jobPercent({ ...progress, transferredBytes: 400 })).toBe(100);
  });
});
