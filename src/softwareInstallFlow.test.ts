import { beforeEach, describe, expect, it, vi } from "vitest";
import { createOperationPlan, installPackage, runOperationDryRun } from "./api";
import { confirmSoftwareInstall, reviewSoftwareInstall } from "./softwareInstallFlow";
import type { DryRunReport, OperationPlanArtifact } from "./types";
vi.mock("./api", () => ({ createOperationPlan: vi.fn(), installPackage: vi.fn(), runOperationDryRun: vi.fn() }));
const plan: OperationPlanArtifact = {
  plan: { planId: "locked-plan", payload: { schemaVersion: 2, action: "installVerifiedDeb", applicationId: "test", packageName: "test", installedVersion: "1", targetVersion: "2", architecture: "amd64", debPath: "/cache/test.deb", sha256: "abc", size: 1, createdAtUnixSeconds: 1, expiresAtUnixSeconds: 200 } },
  planPath: "/plans/locked-plan", missingDependencies: { installable: [], unavailable: [] },
};
const report: DryRunReport = { dryRun: true, planId: "locked-plan", action: "installVerifiedDeb", packageName: "test", installedVersion: "1", targetVersion: "2", architecture: "amd64", sha256: "abc", verified: true, systemModified: false };
beforeEach(() => { vi.resetAllMocks(); vi.spyOn(Date, "now").mockReturnValue(100_000); vi.mocked(createOperationPlan).mockResolvedValue(plan); vi.mocked(runOperationDryRun).mockResolvedValue(report); });
describe("installation review and explicit confirmation", () => {
  it("stops after review without installing", async () => {
    const result = await reviewSoftwareInstall("test", vi.fn());
    expect(runOperationDryRun).toHaveBeenCalledWith("locked-plan");
    expect(result.report).toEqual(report);
    expect(installPackage).not.toHaveBeenCalled();
  });
  it("installs the reviewed plan only after a separate confirmation", async () => {
    const reviewed = await reviewSoftwareInstall("test", vi.fn());
    const progress = vi.fn();
    await confirmSoftwareInstall(reviewed.plan, reviewed.report, progress);
    expect(createOperationPlan).toHaveBeenCalledTimes(1);
    expect(installPackage).toHaveBeenCalledWith("locked-plan", progress);
  });
  it("rejects failed or mismatched reviews", async () => {
    vi.mocked(runOperationDryRun).mockResolvedValue({ ...report, verified: false });
    await expect(reviewSoftwareInstall("test", vi.fn())).rejects.toThrow("未通过");
    await expect(confirmSoftwareInstall(plan, { ...report, planId: "other" }, vi.fn())).rejects.toThrow("未通过");
    expect(installPackage).not.toHaveBeenCalled();
  });
  it("blocks expired plans without creating or installing a replacement", async () => {
    vi.spyOn(Date, "now").mockReturnValue(200_000);
    await expect(confirmSoftwareInstall(plan, report, vi.fn())).rejects.toThrow("已过期");
    expect(installPackage).not.toHaveBeenCalled();
    expect(createOperationPlan).not.toHaveBeenCalled();
  });
});
