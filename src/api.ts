import { invoke } from "@tauri-apps/api/core";
import type { DryRunReport, LocalDebInspection, OperationExecutionReport, OperationPlanArtifact, ScanResult, VscodeDetails, VscodeDownloadPlan, VscodeDownloadResult, WechatDetails } from "./types";

export function scanPackages(): Promise<ScanResult> {
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    return Promise.resolve({
      scannedAtUnixSeconds: Math.floor(Date.now() / 1000),
      warnings: [],
      packages: [
        { packageName: "code", displayName: "Visual Studio Code", vendor: "Microsoft", installedVersion: "1.134.0-1787078834", candidateVersion: "1.134.0-1787078834", architecture: "amd64", sourceKind: "officialRepository", sourceUrl: "https://packages.microsoft.com/repos/code", updateState: "upToDate", homepage: "https://code.visualstudio.com/" },
        { packageName: "google-chrome-stable", displayName: "Google Chrome", vendor: "Google", installedVersion: "151.0.7922.169-1", candidateVersion: "151.0.7922.173-1", architecture: "amd64", sourceKind: "officialRepository", sourceUrl: "https://dl.google.com/linux/chrome-stable/deb", updateState: "updateAvailable", homepage: null },
        { packageName: "chatgpt", displayName: "ChatGPT Desktop", vendor: "OpenAI", installedVersion: "26.818.21641", candidateVersion: "26.818.41705", architecture: "amd64", sourceKind: "officialRepository", sourceUrl: "https://persistent.oaistatic.com/codex-app-prod/linux/deb", updateState: "updateAvailable", homepage: "https://developers.openai.com/codex/app" },
        { packageName: "flclash", displayName: "FlClash", vendor: "FlClash", installedVersion: "0.8.96+2026081701", candidateVersion: null, architecture: "amd64", sourceKind: "localPackage", sourceUrl: null, updateState: "unknown", homepage: "https://github.com/chen08209/FlClash/releases" },
        { packageName: "wechat", displayName: "微信", vendor: "腾讯", installedVersion: "4.1.1.8", candidateVersion: null, architecture: "amd64", sourceKind: "localPackage", sourceUrl: null, updateState: "unknown", homepage: "https://linux.weixin.qq.com/" },
        { packageName: "wemeet", displayName: "腾讯会议", vendor: "腾讯", installedVersion: "3.26.10.401", candidateVersion: null, architecture: "amd64", sourceKind: "localPackage", sourceUrl: null, updateState: "unknown", homepage: "https://meeting.tencent.com/download/" },
      ],
    });
  }
  return invoke<ScanResult>("scan_packages");
}

export function getVscodeDetails(): Promise<VscodeDetails> {
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    return Promise.resolve({
      applicationId: "vscode",
      displayName: "Visual Studio Code",
      packageName: "code",
      installedVersion: "1.134.0-1787078834",
      candidateVersion: "1.134.0-1787078834",
      architecture: "amd64",
      supportLevel: "fullReadOnly",
      trustState: "trusted",
      updateState: "upToDate",
      selectedPath: { kind: "officialRepository", label: "Microsoft 官方 APT 仓库", endpoint: "https://packages.microsoft.com/repos/code", reason: "已配置官方仓库，按照下载优先级直接使用 APT 候选版本。" },
      fallbackEndpoint: "https://update.code.visualstudio.com/latest/linux-deb-x64/stable",
      evidence: [
        { label: "Debian 软件包名", actual: "code", expected: "code", passed: true },
        { label: "系统架构", actual: "amd64", expected: "amd64", passed: true },
        { label: "APT 仓库域名", actual: "https://packages.microsoft.com/repos/code", expected: "packages.microsoft.com", passed: true },
      ],
      verificationPlan: [
        { label: "下载域名", expected: "必须属于 Microsoft 允许列表并使用 HTTPS", state: "planned" },
        { label: "包名", expected: "code", state: "passed" },
        { label: "架构", expected: "amd64", state: "passed" },
        { label: "版本", expected: "1.134.0-1787078834", state: "planned" },
        { label: "SHA-256", expected: "下载完成后与官方索引记录比对", state: "planned" },
      ],
      operationPlan: [
        { order: 1, action: "识别来源", detail: "确认软件包、架构和 Microsoft 官方仓库", state: "complete" },
        { order: 2, action: "解析候选版本", detail: "APT 候选版本 1.134.0-1787078834", state: "complete" },
        { order: 3, action: "下载并校验", detail: "当前已经是最新版本", state: "notRequired" },
        { order: 4, action: "请求安装授权", detail: "当前无需进行特权操作", state: "notRequired" },
      ],
    });
  }
  return invoke<VscodeDetails>("get_vscode_details");
}

export function getWechatDetails(): Promise<WechatDetails> {
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    return Promise.resolve({
      applicationId: "wechat", displayName: "微信", packageName: "wechat",
      installedVersion: "4.1.1.8", websiteVersion: "4.1.1", packageVersion: "4.1.1.8",
      architecture: "amd64", updateState: "upToDate", officialPage: "https://linux.weixin.qq.com/",
      downloadUrl: "https://dldir1v6.qq.com/weixin/Universal/Linux/WeChatLinux_x86_64.deb",
      expectedSize: 212419528, sourceTrusted: true,
      evidence: [
        { label: "官网域名", actual: "linux.weixin.qq.com", expected: "linux.weixin.qq.com", passed: true },
        { label: "下载域名", actual: "dldir1v6.qq.com", expected: "dldir1v6.qq.com", passed: true },
        { label: "Debian 软件包名", actual: "wechat", expected: "wechat", passed: true },
        { label: "软件包架构", actual: "amd64", expected: "amd64", passed: true },
      ],
    });
  }
  return invoke<WechatDetails>("get_wechat_details");
}

const mockDownloadPlan: VscodeDownloadPlan = {
  packageName: "code",
  version: "1.134.0-1787078834",
  architecture: "amd64",
  repositoryUrl: "https://packages.microsoft.com/repos/code",
  downloadUrl: "https://packages.microsoft.com/repos/code/pool/main/c/code/code_1.134.0-1787078834_amd64.deb",
  fileName: "code_1.134.0-1787078834_amd64.deb",
  expectedSize: 238188742,
  expectedSha256: "dcd3a2f52d53df079cd389662ff1fdbeb629938331d3a63655fed929f8d49f19",
  targetPath: "/home/user/.cache/io.github.umanager.app/downloads/code_1.134.0-1787078834_amd64.deb",
};

export function getVscodeDownloadPlan(): Promise<VscodeDownloadPlan> {
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) return Promise.resolve(mockDownloadPlan);
  return invoke<VscodeDownloadPlan>("get_vscode_download_plan");
}

export function downloadVscodePackage(): Promise<VscodeDownloadResult> {
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    return Promise.resolve({
      plan: mockDownloadPlan,
      actualSize: mockDownloadPlan.expectedSize,
      actualSha256: mockDownloadPlan.expectedSha256,
      packageName: "code",
      version: mockDownloadPlan.version,
      architecture: "amd64",
      reusedExistingFile: false,
      verified: true,
    });
  }
  return invoke<VscodeDownloadResult>("download_vscode_package");
}

export function createVscodeOperationPlan(): Promise<OperationPlanArtifact> {
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    const created = Math.floor(Date.now() / 1000);
    return Promise.resolve({
      plan: {
        planId: mockDownloadPlan.expectedSha256,
        payload: {
          schemaVersion: 1,
          action: "installVerifiedDeb",
          applicationId: "vscode",
          packageName: "code",
          installedVersion: "1.133.0-1780000000",
          targetVersion: mockDownloadPlan.version,
          architecture: "amd64",
          debPath: mockDownloadPlan.targetPath,
          sha256: mockDownloadPlan.expectedSha256,
          size: mockDownloadPlan.expectedSize,
          createdAtUnixSeconds: created,
          expiresAtUnixSeconds: created + 15 * 60,
        },
      },
      planPath: `/home/user/.cache/io.github.umanager.app/plans/${mockDownloadPlan.expectedSha256}.json`,
    });
  }
  return invoke<OperationPlanArtifact>("create_vscode_operation_plan");
}

export function runVscodeOperationDryRun(planId: string): Promise<DryRunReport> {
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    return Promise.resolve({
      dryRun: true,
      planId,
      action: "installVerifiedDeb",
      packageName: "code",
      installedVersion: "1.133.0-1780000000",
      targetVersion: mockDownloadPlan.version,
      architecture: "amd64",
      sha256: mockDownloadPlan.expectedSha256,
      verified: true,
      systemModified: false,
    });
  }
  return invoke<DryRunReport>("run_vscode_operation_dry_run", { planId });
}

export function getPendingLocalDeb(): Promise<LocalDebInspection | null> {
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) return Promise.resolve(null);
  return invoke<LocalDebInspection | null>("get_pending_local_deb");
}

export function importPendingLocalDeb(): Promise<LocalDebInspection> {
  return invoke<LocalDebInspection>("import_pending_local_deb");
}

export function createLocalDebOperationPlan(sha256: string): Promise<OperationPlanArtifact> {
  return invoke<OperationPlanArtifact>("create_local_deb_operation_plan", { sha256 });
}

export function runLocalDebDryRun(planId: string): Promise<OperationExecutionReport> {
  return invoke<OperationExecutionReport>("run_local_deb_dry_run", { planId });
}

export function installLocalDeb(planId: string): Promise<OperationExecutionReport> {
  return invoke<OperationExecutionReport>("install_local_deb", { planId });
}
