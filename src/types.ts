export type SourceKind = "officialRepository" | "localPackage";
export type UpdateState = "upToDate" | "updateAvailable" | "unknown";

export interface ManagedPackage {
  packageName: string;
  displayName: string;
  vendor: string;
  installedVersion: string;
  candidateVersion: string | null;
  architecture: string;
  sourceKind: SourceKind;
  sourceUrl: string | null;
  updateState: UpdateState;
  homepage: string | null;
}

export interface ScanResult {
  packages: ManagedPackage[];
  scannedAtUnixSeconds: number;
  warnings: string[];
}

export interface VscodeDetails {
  applicationId: "vscode";
  displayName: string;
  packageName: string;
  installedVersion: string;
  candidateVersion: string | null;
  architecture: string;
  supportLevel: "fullReadOnly";
  trustState: "trusted" | "needsReview";
  updateState: UpdateState;
  selectedPath: {
    kind: "officialRepository" | "stableDownloadEndpoint";
    label: string;
    endpoint: string;
    reason: string;
  };
  fallbackEndpoint: string;
  evidence: Array<{ label: string; actual: string; expected: string; passed: boolean }>;
  verificationPlan: Array<{ label: string; expected: string; state: "passed" | "planned" }>;
  operationPlan: Array<{ order: number; action: string; detail: string; state: "complete" | "planned" | "notRequired" }>;
}

export interface VscodeDownloadPlan {
  packageName: string;
  version: string;
  architecture: string;
  repositoryUrl: string;
  downloadUrl: string;
  fileName: string;
  expectedSize: number;
  expectedSha256: string;
  targetPath: string;
}

export interface VscodeDownloadResult {
  plan: VscodeDownloadPlan;
  actualSize: number;
  actualSha256: string;
  packageName: string;
  version: string;
  architecture: string;
  reusedExistingFile: boolean;
  verified: boolean;
}

export interface WechatDetails {
  applicationId: "wechat";
  displayName: "微信";
  packageName: "wechat";
  installedVersion: string;
  websiteVersion: string;
  packageVersion: string;
  architecture: string;
  updateState: UpdateState;
  officialPage: string;
  downloadUrl: string;
  expectedSize: number;
  sourceTrusted: boolean;
  evidence: Array<{ label: string; actual: string; expected: string; passed: boolean }>;
}

export interface OperationPlanPayload {
  schemaVersion: number;
  action: "installVerifiedDeb" | "installLocalDeb";
  applicationId: string;
  packageName: string;
  installedVersion: string | null;
  targetVersion: string;
  architecture: string;
  debPath: string;
  sha256: string;
  size: number;
  createdAtUnixSeconds: number;
  expiresAtUnixSeconds: number;
}

export interface OperationPlanArtifact {
  plan: { planId: string; payload: OperationPlanPayload };
  planPath: string;
}

export interface DryRunReport {
  dryRun: true;
  planId: string;
  action: "installVerifiedDeb" | "installLocalDeb";
  packageName: string;
  installedVersion: string | null;
  targetVersion: string;
  architecture: string;
  sha256: string;
  verified: boolean;
  systemModified: false;
}

export type LocalDebDisposition = "newInstall" | "upgrade" | "reinstall" | "downgrade" | "unsupportedArchitecture";

export interface LocalDebInspection {
  originalPath: string;
  cachedPath: string | null;
  fileName: string;
  packageName: string;
  version: string;
  architecture: string;
  size: number;
  sha256: string;
  installedVersion: string | null;
  disposition: LocalDebDisposition;
  installAllowed: boolean;
  sourceTrusted: false;
}

export interface OperationExecutionReport {
  dryRun: boolean;
  planId: string;
  action: "installVerifiedDeb" | "installLocalDeb";
  packageName: string;
  installedVersion: string | null;
  targetVersion: string;
  architecture: string;
  sha256: string;
  verified: boolean;
  systemModified: boolean;
}
