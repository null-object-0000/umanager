import { afterEach, expect, it, vi } from "vitest";

afterEach(() => { vi.clearAllTimers(); vi.useRealTimers(); vi.unstubAllGlobals(); vi.resetModules(); });

it("runs the actual preview download queue and supports cancel without a native bridge", async () => {
  vi.useFakeTimers();
  vi.stubGlobal("window", new EventTarget());
  const api = await import("../../src/api");
  const catalog = await api.getSoftwareCatalog();
  expect(catalog.some(app => app.applicationId === "google-chrome")).toBe(true);
  await api.enqueueDownload("google-chrome", null);
  await vi.advanceTimersByTimeAsync(2500);
  const jobs = await api.listDownloads();
  expect(jobs.find(job => job.applicationId === "google-chrome")?.status).toBe("ready");
  await api.enqueueDownload("flclash", null);
  const pending = (await api.listDownloads()).find(job => job.applicationId === "flclash")!;
  await api.cancelDownload(pending.jobId);
  expect((await api.listDownloads()).find(job => job.jobId === pending.jobId)?.status).toBe("canceled");
});
