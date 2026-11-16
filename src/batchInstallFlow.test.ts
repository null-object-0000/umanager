import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { executeBatchUpdate, reviewBatchUpdate } from "./batchApi";
import { installBatchUpdate } from "./batchInstallFlow";
import type { BatchTask } from "./types";
vi.mock("./batchApi", () => ({ executeBatchUpdate: vi.fn(), reviewBatchUpdate: vi.fn() }));
function batch(status: BatchTask["status"] = "awaitingReview"): BatchTask {
  return { id: "batch", status, reviewId: status === "awaitingConfirmation" ? "review" : null, items: [{ status: status === "awaitingConfirmation" ? "reviewed" : "ready", expiresAtUnixSeconds: 200 }] } as BatchTask;
}
beforeEach(() => { vi.resetAllMocks(); vi.spyOn(Date, "now").mockReturnValue(100_000); });
afterEach(() => vi.restoreAllMocks());
describe("one batch update action", () => {
  it("reviews then executes the exact returned review without another click", async () => {
    vi.mocked(reviewBatchUpdate).mockResolvedValue({ revision: 1, batches: [batch("awaitingConfirmation")] });
    await installBatchUpdate(batch());
    expect(reviewBatchUpdate).toHaveBeenCalledWith("batch");
    expect(executeBatchUpdate).toHaveBeenCalledWith("batch", "review");
    expect(vi.mocked(reviewBatchUpdate).mock.invocationCallOrder[0]).toBeLessThan(vi.mocked(executeBatchUpdate).mock.invocationCallOrder[0]);
  });
  it("does not execute when review fails or another action canceled the batch", async () => {
    vi.mocked(reviewBatchUpdate).mockRejectedValueOnce(new Error("review failed"));
    await expect(installBatchUpdate(batch())).rejects.toThrow("review failed");
    vi.mocked(reviewBatchUpdate).mockResolvedValue({ revision: 1, batches: [batch("canceled")] });
    await expect(installBatchUpdate(batch())).rejects.toThrow("未通过");
    expect(executeBatchUpdate).not.toHaveBeenCalled();
  });
  it("re-reviews expired plans and never executes an expired result", async () => {
    const expired = batch("awaitingConfirmation"); expired.items[0].expiresAtUnixSeconds = 99;
    vi.mocked(reviewBatchUpdate).mockResolvedValue({ revision: 2, batches: [expired] });
    await expect(installBatchUpdate(expired)).rejects.toThrow("未通过");
    expect(reviewBatchUpdate).toHaveBeenCalledWith("batch");
    expect(executeBatchUpdate).not.toHaveBeenCalled();
  });
  it("lets a user resume a previously reviewed valid plan", async () => {
    await installBatchUpdate(batch("awaitingConfirmation"));
    expect(reviewBatchUpdate).not.toHaveBeenCalled();
    expect(executeBatchUpdate).toHaveBeenCalledWith("batch", "review");
  });
});
