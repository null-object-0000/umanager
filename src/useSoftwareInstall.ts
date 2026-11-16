import { useEffect, useRef, useState } from "react";
import { executeArchiveOperation, prepareArchiveOperation } from "./api";
import { confirmSoftwareInstall, reviewSoftwareInstall } from "./softwareInstallFlow";
import type { ArchivePlan, DryRunReport, OperationExecutionReport, OperationPlanArtifact, OperationProgressEvent } from "./types";

type InstallPhase = "idle" | "planning" | "dry-run" | "confirming" | "installing" | "done";

// Both software detail drawers share one installation lifecycle. Only the
// execution backend differs; tar.gz never reaches the privileged DEB helper.
export function useSoftwareInstall(applicationId: string | undefined, isArchive: boolean, onInstalled: () => void, onDismissDownload: () => void) {
  const [operationPlan, setOperationPlan] = useState<OperationPlanArtifact | null>(null);
  const [archivePlan, setArchivePlan] = useState<ArchivePlan | null>(null);
  const [dryRun, setDryRun] = useState<DryRunReport | null>(null);
  const [installed, setInstalled] = useState<OperationExecutionReport | null>(null);
  const [archiveInstalled, setArchiveInstalled] = useState<string | null>(null);
  const [installPhase, setInstallPhase] = useState<InstallPhase>("idle");
  const [error, setError] = useState<string | null>(null);
  const [progressEvents, setProgressEvents] = useState<OperationProgressEvent[]>([]);
  const running = useRef(false);
  useEffect(() => {
    setOperationPlan(null); setArchivePlan(null); setDryRun(null); setInstalled(null);
    setArchiveInstalled(null); setInstallPhase("idle"); setError(null); setProgressEvents([]);
  }, [applicationId]);

  const install = async () => {
    if (running.current) return;
    if (!applicationId) { setError("软件源中未找到该应用"); return; }
    running.current = true;
    setError(null);
    try {
      if (isArchive) {
        if (!archivePlan) {
          setInstallPhase("planning");
          setArchivePlan(await prepareArchiveOperation(applicationId, false));
          setInstallPhase("confirming");
          return;
        }
        if (Date.now() / 1000 >= archivePlan.payload.expiresAtUnixSeconds) throw new Error("安装计划已过期，请重新核对后确认安装。");
        setInstallPhase("installing");
        setArchiveInstalled(await executeArchiveOperation(archivePlan.planId));
      } else {
        if (installPhase !== "confirming" || !operationPlan || !dryRun) {
          setOperationPlan(null); setDryRun(null); setProgressEvents([]);
          setInstallPhase("planning");
          const { report } = await reviewSoftwareInstall(applicationId, (plan) => {
            setOperationPlan(plan); setInstallPhase("dry-run");
          });
          setDryRun(report);
          setInstallPhase("confirming");
          return;
        }
        setInstallPhase("installing");
        setInstalled(await confirmSoftwareInstall(operationPlan, dryRun, (event) => setProgressEvents((current) => [...current.slice(-499), event])));
      }
      setInstallPhase("done");
      onInstalled(); onDismissDownload();
    } catch (reason) {
      setError(String(reason)); setOperationPlan(null); setDryRun(null); setArchivePlan(null); setInstallPhase("idle");
    } finally { running.current = false; }
  };
  const cancelConfirmation = () => { setOperationPlan(null); setDryRun(null); setArchivePlan(null); setInstallPhase("idle"); };
  return { operationPlan, archivePlan, dryRun, installed, archiveInstalled, installPhase, error, setError, progressEvents, install, cancelConfirmation };
}
