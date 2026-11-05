import { useEffect, useRef, useState } from "react";
import { createOperationPlan, executeArchiveOperation, installPackage, prepareArchiveOperation, runOperationDryRun } from "./api";
import type { ArchivePlan, OperationExecutionReport, OperationPlanArtifact, OperationProgressEvent } from "./types";

type InstallPhase = "idle" | "planning" | "dry-run" | "confirming" | "installing" | "done";

// Both software detail drawers share one installation lifecycle. Only the
// execution backend differs; tar.gz never reaches the privileged DEB helper.
export function useSoftwareInstall(applicationId: string | undefined, isArchive: boolean, onInstalled: () => void, onDismissDownload: () => void) {
  const [operationPlan, setOperationPlan] = useState<OperationPlanArtifact | null>(null);
  const [archivePlan, setArchivePlan] = useState<ArchivePlan | null>(null);
  const [dryRun, setDryRun] = useState<OperationExecutionReport | null>(null);
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
        setInstallPhase("installing");
        setArchiveInstalled(await executeArchiveOperation(archivePlan.planId));
      } else {
        setInstallPhase("planning");
        const plan = await createOperationPlan(applicationId);
        setOperationPlan(plan);
        setInstallPhase("dry-run");
        setDryRun(await runOperationDryRun(plan.plan.planId));
        setInstallPhase("installing");
        setInstalled(await installPackage(plan.plan.planId, (event) => setProgressEvents((current) => [...current.slice(-499), event])));
      }
      setInstallPhase("done");
      onInstalled(); onDismissDownload();
    } catch (reason) {
      setError(String(reason)); setArchivePlan(null); setInstallPhase("idle");
    } finally { running.current = false; }
  };
  const cancelConfirmation = () => { setArchivePlan(null); setInstallPhase("idle"); };
  return { operationPlan, archivePlan, dryRun, installed, archiveInstalled, installPhase, error, setError, progressEvents, install, cancelConfirmation };
}
