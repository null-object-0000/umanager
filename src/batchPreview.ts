// Browser-only simulation. It calls the existing preview download queue and never
// invokes Tauri, requests authorization, or touches the actual package database.
import { enqueueDownload, getDownloadPlan, getSoftwareCatalog, listDownloads, scanPackages } from "./api";
import { batchActive, batchCandidates, canConfirmBatch } from "./batchTasks";
import type { BatchSnapshot, BatchTask } from "./types";
const key = "umanager-batch-preview";
let snapshot: BatchSnapshot = { revision: 0, batches: [] };
try {
  const saved = localStorage.getItem(key);
  if (saved) snapshot = JSON.parse(saved);
  for (const batch of snapshot.batches) {
    batch.reviewId = null;
    if (batchActive(batch.status)) {
      batch.status = "interrupted"; batch.message = "浏览器预览已重载，请重新检查。";
      for (const item of batch.items) {
        if (item.status === "installing" || item.status === "unknown") { item.status = "unknown"; item.error = "上次安装结果未能确认，请重新检查实际安装状态"; }
        else if (item.status !== "succeeded" && item.status !== "failed") { item.status = "blocked"; item.error = "需重新检查本机状态并生成新计划"; }
      }
    }
  }
} catch { snapshot = { revision: 0, batches: [] }; }
const clone = () => structuredClone(snapshot);
function publish() {
  snapshot.revision++;
  try { localStorage.setItem(key, JSON.stringify(snapshot)); } catch { /* preview only */ }
  window.dispatchEvent(new CustomEvent("preview-batch-tasks", { detail: clone() }));
}
function find(id: string): BatchTask {
  const batch = snapshot.batches.find((b) => b.id === id);
  if (!batch) throw new Error("示例任务不存在");
  return batch;
}
let poll: ReturnType<typeof setInterval> | null = null;
export async function list(): Promise<BatchSnapshot> {
  const jobs = await listDownloads();
  let changed = false;
  for (const batch of snapshot.batches.filter((b) => b.status === "downloading")) {
    for (const item of batch.items.filter((i) => i.status === "downloading")) {
      const job = jobs.find((j) => j.jobId === item.downloadJobId);
      if (job?.status === "ready") { item.status = "ready"; changed = true; }
      if (job?.status === "error" || job?.status === "canceled") { item.status = "failed"; item.error = job.error ?? "下载已取消"; changed = true; }
    }
    if (!batch.items.some((i) => i.status === "downloading")) { batch.status = batch.items.some((i) => i.status === "ready") ? "awaitingReview" : "paused"; batch.message = "示例下载已结束，可以授权复核（仅模拟）。"; changed = true; }
  }
  if (changed) publish();
  if (!snapshot.batches.some((b) => b.status === "downloading") && poll) { clearInterval(poll); poll = null; }
  return clone();
}
export async function start(ids: string[]): Promise<BatchSnapshot> {
  if (snapshot.batches.some((b) => batchActive(b.status))) throw new Error("请先处理或结束当前批量任务");
  const candidates = batchCandidates(await scanPackages(), await getSoftwareCatalog(), ids);
  if (!ids.length || new Set(ids).size !== ids.length || candidates.length !== ids.length) throw new Error("示例清单已变化，请重新检查");
  const items: BatchTask["items"] = [];
  for (const c of candidates) {
    const plan = await getDownloadPlan(c.applicationId);
    const jobs = await enqueueDownload(c.applicationId, plan.version);
    items.push({ applicationId: c.applicationId, packageName: c.package.packageName, displayName: c.package.displayName, installedVersion: c.package.installedVersion, targetVersion: plan.version, architecture: plan.architecture, sha256: plan.expectedSha256 ?? "0".repeat(64), size: plan.expectedSize, downloadUrl: plan.downloadUrl, downloadJobId: jobs.find((j) => j.applicationId === c.applicationId)?.jobId ?? null, status: "downloading", planId: null, expiresAtUnixSeconds: null, error: null, logs: [] });
  }
  snapshot.batches.push({ id: `preview-${Date.now()}`, createdAtUnixSeconds: Math.floor(Date.now() / 1000), updatedAtUnixSeconds: Math.floor(Date.now() / 1000), status: "downloading", items, reviewId: null, message: "浏览器模拟：正在下载示例安装包。" });
  snapshot.batches = snapshot.batches.slice(-100);
  publish();
  poll ??= setInterval(() => { void list(); }, 500);
  return clone();
}
export async function review(id: string): Promise<BatchSnapshot> {
  const batch = find(id);
  if (!["awaitingReview", "awaitingConfirmation"].includes(batch.status)) throw new Error("当前示例不能复核");
  batch.status = "reviewing"; batch.reviewId = null; publish();
  await new Promise((resolve) => setTimeout(resolve, 500));
  for (const item of batch.items.filter((i) => ["ready", "reviewed"].includes(i.status))) { item.status = "reviewed"; item.planId = `preview-${item.applicationId}-${Date.now()}`; item.expiresAtUnixSeconds = Math.floor(Date.now() / 1000) + 900; item.logs.push("浏览器模拟复核通过，未请求管理员权限。"); }
  batch.status = "awaitingConfirmation"; batch.reviewId = `preview-review-${Date.now()}`; batch.message = "示例复核通过；确认后只模拟串行安装。"; publish();
  return clone();
}
export async function execute(id: string, reviewId: string): Promise<BatchSnapshot> {
  const batch = find(id);
  if (batch.reviewId !== reviewId || !canConfirmBatch(batch, Date.now() / 1000)) throw new Error("示例复核已失效");
  batch.status = "installing"; batch.reviewId = null; publish();
  void (async () => {
    for (const item of batch.items.filter((i) => i.status === "reviewed")) {
      item.status = "installing"; publish();
      await new Promise((resolve) => setTimeout(resolve, 700));
      item.status = "succeeded"; item.logs.push("浏览器模拟更新成功，没有操作真实软件。"); publish();
    }
    batch.status = batch.items.every((i) => i.status === "succeeded") ? "completed" : "paused";
    batch.message = "浏览器模拟已结束；真实软件没有变化。"; batch.updatedAtUnixSeconds = Math.floor(Date.now() / 1000); publish();
  })();
  return clone();
}
export async function cancel(id: string): Promise<BatchSnapshot> {
  const batch = find(id);
  if (!["downloading", "awaitingReview", "awaitingConfirmation", "paused", "interrupted"].includes(batch.status)) throw new Error("当前任务不能结束");
  const wasActive = batchActive(batch.status);
  batch.status = "canceled"; batch.reviewId = null; batch.message = "示例批次已结束，记录与共享下载仍保留。";
  if (wasActive) for (const item of batch.items.filter((i) => i.status !== "succeeded" && i.status !== "failed")) item.status = "canceled";
  publish(); return clone();
}
