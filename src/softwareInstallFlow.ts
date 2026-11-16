import { createOperationPlan, installPackage, runOperationDryRun } from "./api";
import type { DryRunReport, OperationPlanArtifact, OperationProgressEvent } from "./types";

export async function reviewSoftwareInstall(applicationId: string, onPlan: (plan: OperationPlanArtifact) => void) {
  const plan = await createOperationPlan(applicationId);
  onPlan(plan);
  const report = await runOperationDryRun(plan.plan.planId);
  if (!report.verified || report.planId !== plan.plan.planId || report.systemModified) {
    throw new Error("安装前复核未通过，请重新复核。");
  }
  return { plan, report };
}

// Confirm the very same immutable plan that was reviewed. Never generate a new
// plan while handling the final confirmation.
export async function confirmSoftwareInstall(plan: OperationPlanArtifact, report: DryRunReport, onProgress: (event: OperationProgressEvent) => void) {
  if (!report.verified || report.planId !== plan.plan.planId || report.systemModified) throw new Error("安装前复核未通过，请重新复核。");
  if (Date.now() / 1000 >= plan.plan.payload.expiresAtUnixSeconds) throw new Error("安装计划已过期，请重新复核后确认安装。");
  return installPackage(plan.plan.planId, onProgress);
}
