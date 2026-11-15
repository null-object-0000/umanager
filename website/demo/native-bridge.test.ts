import { describe, expect, it, vi } from "vitest";
import { getCurrentWebviewWindow, getVersion, invoke } from "./native-bridge";
import type { OperationExecutionReport, OperationPlanArtifact } from "../../src/types";

vi.mock("../../src/api", () => ({ getDownloadPlan: vi.fn(async () => ({
  applicationId: "google-chrome", packageName: "google-chrome-stable", version: "1.2.3", architecture: "amd64",
  fileName: "chrome.deb", expectedSha256: "a".repeat(64), expectedSize: 2048,
})) }));

describe("website-only native simulation", () => {
  it("simulates the install flow without declaring a system change", async () => {
    const artifact = await invoke<OperationPlanArtifact>("create_operation_plan", { applicationId: "google-chrome" });
    expect(artifact.plan.payload.debPath).toBe("/demo/chrome.deb");
    expect(artifact.plan.payload.sha256).toBe("a".repeat(64));
    const args = { planId: artifact.plan.planId };
    const review = await invoke<OperationExecutionReport>("run_operation_dry_run", args);
    const installed = await invoke<OperationExecutionReport>("install_package", args);
    expect(review).toMatchObject({ dryRun: true, systemModified: false, verified: true });
    expect(installed).toMatchObject({ dryRun: false, systemModified: false, targetVersion: "1.2.3" });
  });
  it("rejects unknown commands and arbitrary operation plans", async () => {
    await expect(invoke("remove_managed_package", { path: "/etc" })).rejects.toThrow("网页演示不会访问或修改");
    await expect(invoke("install_package", { planId: "untrusted" })).rejects.toThrow("演示计划不存在");
  });
  it("exposes only a demo version and no desktop window", async () => {
    expect(await getVersion()).toBe("演示");
    expect(() => getCurrentWebviewWindow()).toThrow("没有桌面窗口权限");
    expect(await invoke("list_scripts")).toEqual([]);
  });
});
