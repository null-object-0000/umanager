import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createOperationPlan, installPackage, runOperationDryRun } from "./api";
import { executeReviewedSoftwareInstall, installSoftware, reviewSoftwareInstall } from "./softwareInstallFlow";
import type { DryRunReport, OperationPlanArtifact } from "./types";
vi.mock("./api", () => ({ createOperationPlan: vi.fn(), installPackage: vi.fn(), runOperationDryRun: vi.fn() }));
const plan: OperationPlanArtifact = {
  plan: { planId: "locked-plan", payload: { schemaVersion: 2, action: "installVerifiedDeb", applicationId: "test", packageName: "test", installedVersion: "1", targetVersion: "2", architecture: "amd64", debPath: "/cache/test.deb", sha256: "abc", size: 1, createdAtUnixSeconds: 1, expiresAtUnixSeconds: 200 } },
  planPath: "/plans/locked-plan", missingDependencies: { installable: [], unavailable: [] },
};
const report: DryRunReport = { dryRun: true, planId: "locked-plan", action: "installVerifiedDeb", packageName: "test", installedVersion: "1", targetVersion: "2", architecture: "amd64", sha256: "abc", verified: true, systemModified: false };
beforeEach(() => { vi.resetAllMocks(); vi.spyOn(Date, "now").mockReturnValue(100_000); vi.mocked(createOperationPlan).mockResolvedValue(plan); vi.mocked(runOperationDryRun).mockResolvedValue(report); });
afterEach(() => vi.restoreAllMocks());
describe("installation review and execution", () => {
  it("stops after review without installing", async () => {
    const result = await reviewSoftwareInstall("test", vi.fn());
    expect(runOperationDryRun).toHaveBeenCalledWith("locked-plan");
    expect(result.report).toEqual(report);
    expect(installPackage).not.toHaveBeenCalled();
  });
  it("one install action reviews and executes the same immutable plan", async () => {
    const progress = vi.fn();
    const onReviewed = vi.fn();
    await installSoftware("test", vi.fn(), onReviewed, progress);
    expect(onReviewed).toHaveBeenCalledWith(report);
    expect(runOperationDryRun).toHaveBeenCalledWith("locked-plan");
    expect(vi.mocked(runOperationDryRun).mock.invocationCallOrder[0]).toBeLessThan(vi.mocked(installPackage).mock.invocationCallOrder[0]);
    expect(createOperationPlan).toHaveBeenCalledTimes(1);
    expect(installPackage).toHaveBeenCalledWith("locked-plan", progress);
  });
  it("rejects failed or mismatched reviews", async () => {
    vi.mocked(runOperationDryRun).mockResolvedValue({ ...report, verified: false });
    await expect(installSoftware("test", vi.fn(), vi.fn(), vi.fn())).rejects.toThrow("未通过");
    await expect(executeReviewedSoftwareInstall(plan, { ...report, planId: "other" }, vi.fn())).rejects.toThrow("未通过");
    expect(installPackage).not.toHaveBeenCalled();
  });
  it("blocks a plan that expires during review", async () => {
    vi.spyOn(Date, "now").mockReturnValue(200_000);
    await expect(installSoftware("test", vi.fn(), vi.fn(), vi.fn())).rejects.toThrow("已过期");
    expect(installPackage).not.toHaveBeenCalled();
    expect(createOperationPlan).toHaveBeenCalledTimes(1);
  });
});
