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

// Execute the exact immutable plan that passed the privileged review.
export async function executeReviewedSoftwareInstall(plan: OperationPlanArtifact, report: DryRunReport, onProgress: (event: OperationProgressEvent) => void) {
  if (!report.verified || report.planId !== plan.plan.planId || report.systemModified) throw new Error("安装前复核未通过，请重新复核。");
  if (Date.now() / 1000 >= plan.plan.payload.expiresAtUnixSeconds) throw new Error("安装计划已过期，请重试。");
  return installPackage(plan.plan.planId, onProgress);
}

// Called only by an explicit Install / Update action. Downloads never invoke it.
export async function installSoftware(applicationId: string, onPlan: (plan: OperationPlanArtifact) => void, onReviewed: (report: DryRunReport) => void, onProgress: (event: OperationProgressEvent) => void) {
  const { plan, report } = await reviewSoftwareInstall(applicationId, onPlan);
  onReviewed(report);
  return executeReviewedSoftwareInstall(plan, report, onProgress);
}
