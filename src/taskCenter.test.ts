import { describe, expect, it } from "vitest";
import { batchNeedsAttention, matchesBatchFilter, matchesDownloadFilter, standaloneDownloads } from "./taskCenter";
import type { BatchTask, DownloadJob } from "./types";
const batch = (status: BatchTask["status"]) => ({ status, items: [{ downloadJobId: "owned" }] } as BatchTask);
const job = (status: DownloadJob["status"], jobId = "single") => ({ status, jobId } as DownloadJob);
describe("task center", () => {
  it("surfaces review, confirmation and interrupted tasks as attention", () => {
    for (const status of ["awaitingReview", "awaitingConfirmation", "paused", "interrupted"] as const) {
      expect(batchNeedsAttention(status)).toBe(true);
      expect(matchesBatchFilter(batch(status), "attention")).toBe(true);
      expect(matchesBatchFilter(batch(status), "active")).toBe(false);
    }
    expect(matchesBatchFilter(batch("installing"), "active")).toBe(true);
  });
  it("includes canceled and unsuccessful batches in history", () => {
    for (const status of ["completed", "canceled", "paused", "interrupted"] as const) expect(matchesBatchFilter(batch(status), "history")).toBe(true);
    expect(matchesBatchFilter(batch("awaitingConfirmation"), "history")).toBe(false);
  });
  it("shows ready downloads and errors as actionable, downloads as running", () => {
    expect(matchesDownloadFilter(job("ready"), "attention")).toBe(true);
    expect(matchesDownloadFilter(job("error"), "attention")).toBe(true);
    expect(matchesDownloadFilter(job("ready"), "active")).toBe(false);
    expect(matchesDownloadFilter(job("verifying"), "active")).toBe(true);
    expect(matchesDownloadFilter(job("canceled"), "history")).toBe(true);
  });
  it("does not duplicate a batch download, even after batch completion", () => {
    const jobs = [job("ready", "owned"), job("ready")];
    expect(standaloneDownloads(jobs, [batch("downloading")])).toEqual([jobs[1]]);
    expect(standaloneDownloads(jobs, [batch("completed")])).toEqual([jobs[1]]);
    expect(standaloneDownloads(jobs, [batch("canceled")])).toEqual(jobs);
  });
});
