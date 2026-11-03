import { describe, expect, it } from "vitest";
import { batchActive, batchCandidates, canConfirmBatch, reviewExpiresAt, unfinishedIds } from "./batchTasks";
import type { BatchTask, CatalogApplication, ManagedPackage, ScanResult } from "./types";
function pkg(name: string): ManagedPackage {
  return { packageName: name, displayName: name, vendor: "test", installedVersion: "1", candidateVersion: "2", architecture: "amd64", sourceKind: "officialWebsite", sourceUrl: null, updateState: "updateAvailable", homepage: null };
}
function app(id: string, packageName = id): CatalogApplication {
  return { applicationId: id, packageName, displayName: id, vendor: "test", architecture: "amd64", homepage: null, icon: null, accentColor: null, removable: true, source: { kind: "stableDownloadEndpoint" } };
}
function scan(packages: ManagedPackage[]): ScanResult { return { packages, scannedAtUnixSeconds: 1, warnings: [] }; }
function batch(): BatchTask {
  return { id: "batch", createdAtUnixSeconds: 1, updatedAtUnixSeconds: 2, status: "awaitingConfirmation", reviewId: "digest", message: "", items: ["a", "b"].map((id, index) => ({ applicationId: id, packageName: id, displayName: id, installedVersion: "1", targetVersion: "2", architecture: "amd64", sha256: "a".repeat(64), size: 100, downloadUrl: "https://example.com/app.deb", downloadJobId: `job-${id}`, status: "reviewed", planId: id.repeat(64), expiresAtUnixSeconds: 100 + index * 20, error: null, logs: [] })) };
}
describe("batch update selection", () => {
  it("only includes installed managed updates with downloadable sources", () => {
    const installed = [pkg("a"), pkg("u-manager"), pkg("unknown"), { ...pkg("current"), updateState: "upToDate" as const }, pkg("manual"), { ...pkg("missing"), candidateVersion: null }];
    const manual = app("manual"); manual.source.kind = "browserImport";
    expect(batchCandidates(scan(installed), [app("a"), app("umanager", "u-manager"), app("current"), manual, app("missing")]).map((c) => c.applicationId)).toEqual(["a"]);
  });
  it("keeps both installed variants and selects only the requested retry IDs", () => {
    const first = app("domestic"), second = app("global"); first.variantGroup = second.variantGroup = "editor";
    const packages = scan([pkg("domestic"), pkg("global")]);
    expect(batchCandidates(packages, [first, second])).toHaveLength(2);
    expect(batchCandidates(packages, [first, second], ["global"]).map((c) => c.applicationId)).toEqual(["global"]);
  });
  it("does not offer a now-current item when checking a retry", () => {
    expect(batchCandidates(scan([{ ...pkg("a"), updateState: "upToDate" }]), [app("a")], ["a"])).toEqual([]);
  });
  it("retries every unfinished item while preserving successful records", () => {
    const task = batch(); task.items[0].status = "succeeded"; task.items[1].status = "blocked";
    expect(unfinishedIds(task)).toEqual(["b"]);
    expect(task.items[0].status).toBe("succeeded");
  });
});
describe("batch confirmation", () => {
  it("expires at the earliest reviewed plan and blocks stale confirmation", () => {
    const task = batch(); expect(reviewExpiresAt(task)).toBe(100);
    expect(canConfirmBatch(task, 99)).toBe(true); expect(canConfirmBatch(task, 101)).toBe(false);
  });
  it("requires the exact confirmation phase, a review ID and nonempty reviewed plans", () => {
    const task = batch(); task.status = "installing"; expect(canConfirmBatch(task, 99)).toBe(false);
    task.status = "awaitingConfirmation"; task.reviewId = null; expect(canConfirmBatch(task, 99)).toBe(false);
    task.reviewId = "digest"; task.items[0].expiresAtUnixSeconds = null; expect(canConfirmBatch(task, 99)).toBe(false);
    task.items = []; expect(canConfirmBatch(task, 99)).toBe(false);
  });
  it("excludes failed downloads from the reviewed subset expiry", () => {
    const task = batch(); task.items[0].status = "failed"; task.items[0].expiresAtUnixSeconds = null;
    expect(reviewExpiresAt(task)).toBe(120); expect(canConfirmBatch(task, 119)).toBe(true);
  });
  it("treats interrupted and paused tasks as history needing a fresh check", () => {
    expect(batchActive("installing")).toBe(true); expect(batchActive("reviewing")).toBe(true);
    expect(batchActive("interrupted")).toBe(false); expect(batchActive("paused")).toBe(false);
    expect(batchActive("canceled")).toBe(false); expect(batchActive("completed")).toBe(false);
  });
});
