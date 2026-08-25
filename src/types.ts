export type SourceKind = "officialRepository" | "officialWebsite" | "localPackage";
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

export interface CatalogSource {
  kind: "aptRepository" | "stableDownloadEndpoint" | "releaseApi" | "browserImport";
  [key: string]: unknown;
}

export interface CatalogApplication {
  applicationId: string;
  packageName: string;
  displayName: string;
  vendor: string;
  architecture: string;
  homepage: string | null;
  icon: string | null;
  accentColor: string | null;
  removable: boolean;
  source: CatalogSource;
}

export interface Evidence {
  label: string;
  actual: string;
  expected: string;
  passed: boolean;
}

export interface ApplicationDetails {
  applicationId: string;
  displayName: string;
  packageName: string;
  vendor: string;
  architecture: string;
  sourceKind: "officialRepository" | "officialWebsite";
  sourceUrl: string;
  installedVersion: string | null;
  candidateVersion: string | null;
  updateState: UpdateState;
  websiteVersion: string | null;
  expectedSize: number | null;
  sha256: string | null;
  metadataBytes: number | null;
  releaseTag: string | null;
  assetName: string | null;
  trusted: boolean;
  evidence: Evidence[];
}

export interface DownloadPlan {
  applicationId: string;
  packageName: string;
  version: string;
  architecture: string;
  sourceKind: "officialRepository" | "officialWebsite";
  repositoryUrl: string | null;
  downloadUrl: string;
  fileName: string;
  expectedSize: number;
  expectedSha256: string | null;
  targetPath: string;
  releaseTag: string | null;
  assetName: string | null;
  websiteVersion: string | null;
}

export interface DownloadResult {
  plan: DownloadPlan;
  actualSize: number;
  actualSha256: string;
  packageName: string;
  version: string;
  architecture: string;
  reusedExistingFile: boolean;
  verified: boolean;
}

export interface DownloadProgress {
  packageName: string;
  phase: "downloading" | "verifying" | "completed";
  transferredBytes: number;
  totalBytes: number;
  bytesPerSecond: number;
}

export interface InstallableApplication {
  applicationId: string;
  packageName: string;
  displayName: string;
  vendor: string;
  architecture: string;
  sourceKind: "officialRepository" | "officialWebsite";
  installedVersion: string | null;
  candidateVersion: string | null;
  installAvailable: boolean;
  unavailableReason: string | null;
  downloadPlan: DownloadPlan | null;
}

export interface OperationPlanPayload {
  schemaVersion: number;
  action: "installVerifiedDeb" | "installVerifiedWebsiteDeb" | "installLocalDeb";
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
  action: "installVerifiedDeb" | "installVerifiedWebsiteDeb" | "installLocalDeb";
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
  action: "installVerifiedDeb" | "installVerifiedWebsiteDeb" | "installLocalDeb";
  packageName: string;
  installedVersion: string | null;
  targetVersion: string;
  architecture: string;
  sha256: string;
  verified: boolean;
  systemModified: boolean;
}

export interface RemovalPlanPayload {
  schemaVersion: number;
  action: "removeManagedPackage" | "removeUmanager";
  applicationId: string;
  packageName: string;
  installedVersion: string;
  architecture: string;
  createdAtUnixSeconds: number;
  expiresAtUnixSeconds: number;
}

export interface RemovalPlanArtifact {
  plan: { planId: string; payload: RemovalPlanPayload };
  planPath: string;
}

export interface RemovalExecutionReport {
  dryRun: boolean;
  planId: string;
  action: "removeManagedPackage" | "removeUmanager";
  packageName: string;
  installedVersion: string;
  architecture: string;
  verified: boolean;
  systemModified: boolean;
}

export interface InstallationInfo {
  appVersion: string;
  installationKind: "debianPackage" | "portable" | "development";
  packageName: string | null;
  packageVersion: string | null;
  architecture: string | null;
  executablePath: string;
  canSelfRemove: boolean;
}

export interface OperationProgressEvent {
  planId: string;
  kind: "phase" | "log" | "warning" | "completed";
  stream: "system" | "stdout" | "stderr";
  message: string;
}

export interface DevToolchain {
  toolchainId: string;
  displayName: string;
  vendor: string;
  homepage: string;
  icon: string | null;
  accentColor: string | null;
  manager: string;
  managerHome: string;
  managerScript: string;
  versionsDirectory: string;
}

export interface DevVersion {
  version: string;
  isDefault: boolean;
  isLts: boolean;
  ltsName: string | null;
}

export interface DevToolchainState {
  toolchainId: string;
  displayName: string;
  vendor: string;
  homepage: string;
  manager: string;
  managerFound: boolean;
  managerHome: string | null;
  managerVersion: string | null;
  defaultVersion: string | null;
  installedVersions: DevVersion[];
}

export interface DevRelease {
  version: string;
  major: number;
  lts: string;
  latestLts: boolean;
}

export interface DevOperationReport {
  toolchainId: string;
  action: string;
  version: string;
  success: boolean;
  message: string;
}

export interface DevOperationProgress {
  toolchainId: string;
  phase: "running" | "completed";
  stream: "system" | "stdout" | "stderr";
  message: string;
}

