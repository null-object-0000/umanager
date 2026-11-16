import { useEffect, useRef, useState } from "react";
import { executeArchiveOperation, prepareArchiveOperation } from "./api";
import { installSoftware } from "./softwareInstallFlow";
import type { ArchivePlan, DryRunReport, OperationExecutionReport, OperationPlanArtifact, OperationProgressEvent } from "./types";

type InstallPhase = "idle" | "planning" | "dry-run" | "installing" | "done";

// Both software detail drawers share one installation lifecycle. Only the
// execution backend differs; tar.gz never reaches the privileged DEB helper.
export function useSoftwareInstall(applicationId: string | undefined, isArchive: boolean, onInstalled: () => void, onDismissDownload: () => void) {
  const [operationPlan, setOperationPlan] = useState<OperationPlanArtifact | null>(null);
  const [archivePlan, setArchivePlan] = useState<ArchivePlan | null>(null);
  const [dryRun, setDryRun] = useState<DryRunReport | null>(null);
  const [installed, setInstalled] = useState<OperationExecutionReport | null>(null);
  const [installPhase, setInstallPhase] = useState<InstallPhase>("idle");
  const [error, setError] = useState<string | null>(null);
  const [progressEvents, setProgressEvents] = useState<OperationProgressEvent[]>([]);
  const running = useRef(false);
  useEffect(() => {
    setOperationPlan(null); setArchivePlan(null); setDryRun(null); setInstalled(null);
    setInstallPhase("idle"); setError(null); setProgressEvents([]);
  }, [applicationId]);

  const install = async () => {
    if (running.current) return;
    if (!applicationId) { setError("软件源中未找到该应用"); return; }
    running.current = true;
    setError(null);
    try {
      setOperationPlan(null); setArchivePlan(null); setDryRun(null); setProgressEvents([]);
      setInstallPhase("planning");
      if (isArchive) {
        const plan = await prepareArchiveOperation(applicationId, false);
        setArchivePlan(plan);
        if (Date.now() / 1000 >= plan.payload.expiresAtUnixSeconds) throw new Error("安装计划已过期，请重试。");
        setInstallPhase("installing");
        await executeArchiveOperation(plan.planId);
      } else {
        setInstalled(await installSoftware(applicationId, (plan) => {
          setOperationPlan(plan); setInstallPhase("dry-run");
        }, (report) => {
          setDryRun(report); setInstallPhase("installing");
        }, (event) => setProgressEvents((current) => [...current.slice(-499), event])));
      }
      setInstallPhase("done");
      onInstalled(); onDismissDownload();
    } catch (reason) {
      setError(String(reason)); setInstallPhase("idle");
    } finally { running.current = false; }
  };
  return { operationPlan, archivePlan, dryRun, installed, installPhase, error, setError, progressEvents, install };
}
