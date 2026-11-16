import { executeBatchUpdate, reviewBatchUpdate } from "./batchApi";
import { canConfirmBatch } from "./batchTasks";
import type { BatchTask } from "./types";

// Only invoked by the user pressing Update; background snapshots never execute.
export async function installBatchUpdate(batch: BatchTask) {
  let reviewed = batch;
  if (!canConfirmBatch(reviewed, Date.now() / 1000)) {
    const snapshot = await reviewBatchUpdate(batch.id);
    const next = snapshot.batches.find((item) => item.id === batch.id);
    if (!next) throw new Error("更新任务不存在，请刷新。");
    reviewed = next;
  }
  if (!canConfirmBatch(reviewed, Date.now() / 1000)) throw new Error("更新前复核未通过，请检查任务详情。");
  return executeBatchUpdate(reviewed.id, reviewed.reviewId!);
}
