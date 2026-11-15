import type { OperationPlanArtifact, OperationExecutionReport } from "../../src/types";

const plans = new Map<string, OperationPlanArtifact>();

/** A browser-only simulation, with no Tauri transport or network access. */
export async function invoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  if (command === "list_scripts") return [] as T;
  if (command === "create_operation_plan") {
    const { getDownloadPlan } = await import("../../src/api");
    const applicationId = String(args.applicationId);
    const download = await getDownloadPlan(applicationId);
    const created = Math.floor(Date.now() / 1000);
    const planId = "website-demo-" + applicationId;
    const artifact: OperationPlanArtifact = {
      plan: { planId, payload: {
        schemaVersion: 2, action: "installVerifiedDeb", applicationId,
        packageName: download.packageName, installedVersion: null,
        targetVersion: download.version, architecture: download.architecture,
        debPath: "/demo/" + download.fileName, sha256: download.expectedSha256 ?? "0".repeat(64),
        size: download.expectedSize, createdAtUnixSeconds: created, expiresAtUnixSeconds: created + 900,
      } },
      planPath: "/demo/plan.json", missingDependencies: { installable: [], unavailable: [] },
    };
    plans.set(planId, artifact);
    return artifact as T;
  }
  if (command === "run_operation_dry_run" || command === "install_package") {
    const artifact = plans.get(String(args.planId));
    if (!artifact) throw new Error("演示计划不存在，请重新选择软件。");
    const payload = artifact.plan.payload;
    const result: OperationExecutionReport = {
      dryRun: command === "run_operation_dry_run", planId: artifact.plan.planId,
      action: payload.action, packageName: payload.packageName, installedVersion: payload.installedVersion,
      targetVersion: payload.targetVersion, architecture: payload.architecture,
      sha256: payload.sha256, verified: true, systemModified: false,
    };
    return result as T;
  }
  // Unimplemented native-only features fail explicitly; never call a bridge,
  // shell, local service, package URL, or user-configured API endpoint.
  throw new Error("此操作请在 Ubuntu 桌面版中体验；网页演示不会访问或修改你的电脑。");
}

export async function listen<T>(_event: string, _handler: (event: { payload: T }) => void): Promise<() => void> {
  return () => {};
}
export async function getVersion(): Promise<string> { return "演示"; }
export function getCurrentWebviewWindow() {
  throw new Error("网页演示没有桌面窗口权限。");
}
