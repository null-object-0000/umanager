import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { BatchSnapshot } from "./types";
const isPreview = () => import.meta.env.DEV && !("__TAURI_INTERNALS__" in window);
export const batchPreview = isPreview;
export async function listBatchTasks(): Promise<BatchSnapshot> {
  if (isPreview()) return (await import("./batchPreview")).list();
  return invoke("list_batch_tasks");
}
export async function startBatchUpdate(applicationIds: string[]): Promise<BatchSnapshot> {
  if (isPreview()) return (await import("./batchPreview")).start(applicationIds);
  return invoke("start_batch_update", { applicationIds });
}
export async function reviewBatchUpdate(batchId: string): Promise<BatchSnapshot> {
  if (isPreview()) return (await import("./batchPreview")).review(batchId);
  return invoke("review_batch_update", { batchId });
}
export async function executeBatchUpdate(batchId: string, reviewId: string): Promise<BatchSnapshot> {
  if (isPreview()) return (await import("./batchPreview")).execute(batchId, reviewId);
  return invoke("execute_batch_update", { batchId, reviewId });
}
export async function cancelBatchUpdate(batchId: string): Promise<BatchSnapshot> {
  if (isPreview()) return (await import("./batchPreview")).cancel(batchId);
  return invoke("cancel_batch_update", { batchId });
}
export async function onBatchTasksChanged(callback: (snapshot: BatchSnapshot) => void): Promise<() => void> {
  if (isPreview()) {
    const handler = (event: Event) => callback((event as CustomEvent<BatchSnapshot>).detail);
    window.addEventListener("preview-batch-tasks", handler);
    return () => window.removeEventListener("preview-batch-tasks", handler);
  }
  return listen<BatchSnapshot>("batch-tasks-changed", (event) => callback(event.payload));
}
