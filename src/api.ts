import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ApplicationDetails, CatalogApplication, CategoryCatalog, ClipboardEntry, DevOperationProgress, DevOperationReport, DevRelease, DevTool, DevToolchain, DevToolchainState, DevToolProgress, DevToolReport, DevToolState, DownloadPlan, DownloadProgress, DownloadResult, DryRunReport, FeedStatus, InstallableApplication, InstallationInfo, LlmSettings, LlmTranslateDelta, LocalDebInspection, NetworkSettings, OperationExecutionReport, OperationPlanArtifact, OperationProgressEvent, RemovalExecutionReport, RemovalPlanArtifact, ScanResult, ScriptDefinition, ScriptProgressEvent, ScriptRunReport, SessionInfo } from "./types";

const isMock = () => import.meta.env.DEV && !("__TAURI_INTERNALS__" in window);

async function invokeWithOperationProgress<T>(command: string, planId: string, onProgress?: (event: OperationProgressEvent) => void): Promise<T> {
  const unlisten = onProgress ? await listen<OperationProgressEvent>("operation-progress", ({ payload }) => {
    if (payload.planId === planId) onProgress(payload);
  }) : null;
  try {
    return await invoke<T>(command, { planId });
  } finally {
    unlisten?.();
  }
}

const mockCatalog: CatalogApplication[] = [
  { applicationId: "vscode", packageName: "code", displayName: "Visual Studio Code", vendor: "Microsoft", architecture: "amd64", homepage: "https://code.visualstudio.com/", icon: "vscode", accentColor: "#2b78bd", removable: true, source: { kind: "aptRepository", repositoryUrl: "https://packages.microsoft.com/repos/code", repositoryHosts: ["packages.microsoft.com"] } },
  { applicationId: "google-chrome", packageName: "google-chrome-stable", displayName: "Google Chrome", vendor: "Google", architecture: "amd64", homepage: "https://www.google.com/chrome/", icon: "google-chrome", accentColor: "#4285f4", removable: true, source: { kind: "aptRepository", repositoryUrl: "https://dl.google.com/linux/chrome-stable/deb", repositoryHosts: ["dl.google.com"] } },
  { applicationId: "chatgpt", packageName: "chatgpt", displayName: "ChatGPT Desktop", vendor: "OpenAI", architecture: "amd64", homepage: "https://developers.openai.com/codex/app", icon: "chatgpt", accentColor: "#171918", removable: true, source: { kind: "aptRepository", repositoryUrl: "https://persistent.oaistatic.com/codex-app-prod/linux/deb", repositoryHosts: ["persistent.oaistatic.com"] } },
  { applicationId: "flclash", packageName: "flclash", displayName: "FlClash", vendor: "FlClash", architecture: "amd64", homepage: "https://github.com/chen08209/FlClash/releases", icon: "flclash", accentColor: "#7c5ce5", removable: true, source: { kind: "releaseApi", releaseApiUrl: "https://api.github.com/repos/chen08209/FlClash/releases/latest", releaseApiHosts: ["api.github.com"], assetNamePattern: "FlClash-{tagVersion}-linux-amd64.deb", stripTagPrefix: "v", assetDownloadHosts: ["github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com"] } },
  { applicationId: "wechat", packageName: "wechat", displayName: "微信", vendor: "腾讯", architecture: "amd64", homepage: "https://linux.weixin.qq.com/", icon: "wechat", accentColor: "#22ad38", removable: true, source: { kind: "stableDownloadEndpoint", officialPageUrl: "https://linux.weixin.qq.com/", officialPageHosts: ["linux.weixin.qq.com"], downloadUrl: "https://dldir1v6.qq.com/weixin/Universal/Linux/WeChatLinux_x86_64.deb", downloadHosts: ["dldir1v6.qq.com"], pageVersionMarker: "main-section__bd-version\"", downloadLinkFileName: "WeChatLinux_x86_64.deb", pageVersionSegments: 3 } },
  { applicationId: "wemeet", packageName: "wemeet", displayName: "腾讯会议", vendor: "腾讯", architecture: "amd64", homepage: "https://meeting.tencent.com/download/", icon: "wemeet", accentColor: "#2878ff", removable: true, source: { kind: "versionEndpoint", versionEndpointUrl: "https://meeting.tencent.com/web-service/query-download-info", versionEndpointHosts: ["meeting.tencent.com"], payloadKind: "json", versionField: "info-list.0.version", downloadUrlField: "info-list.0.url", query: { q: [{ "package-type": "app", channel: "0300000000", platform: "linux", arch: "x86_64", decorators: ["deb"] }], nonce: "{nonce}", rnds: "{rnds}" }, downloadHosts: ["updatecdn.meeting.qq.com"] } },
];

const mockPlans: Record<string, DownloadPlan> = {
  vscode: { applicationId: "vscode", packageName: "code", version: "1.134.0-1787078834", architecture: "amd64", sourceKind: "officialRepository", repositoryUrl: "https://packages.microsoft.com/repos/code", downloadUrl: "https://packages.microsoft.com/repos/code/pool/main/c/code/code_1.134.0-1787078834_amd64.deb", fileName: "code_1.134.0-1787078834_amd64.deb", expectedSize: 238188742, expectedSha256: "dcd3a2f52d53df079cd389662ff1fdbeb629938331d3a63655fed929f8d49f19", targetPath: "/home/user/.cache/io.github.umanager.app/downloads/code.deb", releaseTag: null, assetName: null, websiteVersion: null },
  "google-chrome": { applicationId: "google-chrome", packageName: "google-chrome-stable", version: "151.0.7922.173-1", architecture: "amd64", sourceKind: "officialRepository", repositoryUrl: "https://dl.google.com/linux/chrome-stable/deb", downloadUrl: "https://dl.google.com/linux/chrome-stable/deb/pool/main/g/google-chrome-stable/google-chrome-stable_151.0.7922.173-1_amd64.deb", fileName: "google-chrome-stable_151.0.7922.173-1_amd64.deb", expectedSize: 140077524, expectedSha256: "8".repeat(64), targetPath: "/home/user/.cache/io.github.umanager.app/downloads/google-chrome-stable.deb", releaseTag: null, assetName: null, websiteVersion: null },
  chatgpt: { applicationId: "chatgpt", packageName: "chatgpt", version: "26.818.61809", architecture: "amd64", sourceKind: "officialRepository", repositoryUrl: "https://persistent.oaistatic.com/codex-app-prod/linux/deb", downloadUrl: "https://persistent.oaistatic.com/codex-app-prod/linux/deb/pool/main/c/chatgpt/chatgpt_26.818.61809_amd64.deb", fileName: "chatgpt_26.818.61809_amd64.deb", expectedSize: 388572198, expectedSha256: "1".repeat(64), targetPath: "/home/user/.cache/io.github.umanager.app/downloads/chatgpt.deb", releaseTag: null, assetName: null, websiteVersion: null },
  wechat: { applicationId: "wechat", packageName: "wechat", version: "4.1.2.1", architecture: "amd64", sourceKind: "officialWebsite", repositoryUrl: null, downloadUrl: "https://dldir1v6.qq.com/weixin/Universal/Linux/WeChatLinux_x86_64.deb", fileName: "wechat-4.1.2.1.deb", expectedSize: 212419528, expectedSha256: null, targetPath: "/home/user/.cache/io.github.umanager.app/downloads/wechat.deb", releaseTag: null, assetName: null, websiteVersion: "4.1.2" },
  flclash: { applicationId: "flclash", packageName: "flclash", version: "0.8.97+2026082401", architecture: "amd64", sourceKind: "officialWebsite", repositoryUrl: null, downloadUrl: "https://github.com/chen08209/FlClash/releases/download/v0.8.97/FlClash-0.8.97-linux-amd64.deb", fileName: "flclash-0.8.97.deb", expectedSize: 42400000, expectedSha256: "b24f5aa073952fabfb5b65d67f2800c824fb6a5bce8663524382dc7319d3864c", targetPath: "/home/user/.cache/io.github.umanager.app/downloads/flclash.deb", releaseTag: "v0.8.97", assetName: "FlClash-0.8.97-linux-amd64.deb", websiteVersion: "0.8.97" },
};

function mockDetails(applicationId: string): ApplicationDetails {
  const entry = mockCatalog.find((item) => item.applicationId === applicationId);
  const plan = mockPlans[applicationId];
  const base = {
    applicationId,
    displayName: entry?.displayName ?? applicationId,
    packageName: entry?.packageName ?? applicationId,
    vendor: entry?.vendor ?? "",
    architecture: entry?.architecture ?? "amd64",
    trusted: true,
  };
  const installed: Record<string, string | null> = { vscode: "1.134.0-1787078834", "google-chrome": "151.0.7922.169-1", chatgpt: "26.818.21641", wechat: "4.1.1.8", flclash: "0.8.96+2026081701" };
  if (!plan) throw new Error(`应用 ${applicationId} 没有可用的下载源`);
  const website = plan.sourceKind === "officialWebsite";
  const updateState = installed[applicationId] && installed[applicationId] !== plan.version ? "updateAvailable" : "upToDate";
  return {
    ...base,
    sourceKind: plan.sourceKind,
    sourceUrl: plan.repositoryUrl ?? plan.downloadUrl,
    installedVersion: installed[applicationId] ?? null,
    candidateVersion: plan.version,
    updateState,
    websiteVersion: plan.websiteVersion,
    expectedSize: plan.expectedSize,
    sha256: plan.expectedSha256,
    metadataBytes: website ? 592 : null,
    releaseTag: plan.releaseTag,
    assetName: plan.assetName,
    releaseNotes: applicationId === "flclash"
      ? "## 0.8.97\n\n- 修复托盘与代理规则导入的问题\n- 优化订阅刷新与连接稳定性\n- 升级内置 Clash 内核"
      : null,
    releaseNotesUrl: applicationId === "flclash"
      ? "https://github.com/chen08209/FlClash/releases/tag/v0.8.97"
      : null,
    evidence: [
      { label: website ? "下载域名" : "APT 仓库域名", actual: plan.repositoryUrl ?? plan.downloadUrl, expected: plan.repositoryUrl ?? plan.downloadUrl, passed: true },
      { label: "Debian 软件包名", actual: plan.packageName, expected: plan.packageName, passed: true },
      { label: "软件包架构", actual: plan.architecture, expected: "amd64", passed: true },
    ],
  };
}

export function getInstallationInfo(): Promise<InstallationInfo> {
  if (isMock()) {
    return Promise.resolve({ appVersion: "0.1.0", installationKind: "development", packageName: null, packageVersion: null, architecture: null, executablePath: "/path/to/umanager/src-tauri/target/debug/umanager", canSelfRemove: false });
  }
  return invoke<InstallationInfo>("get_installation_info");
}

export function restartApp(): Promise<void> {
  if (isMock()) return Promise.resolve();
  return invoke<void>("restart_app");
}

export function notifyDownloadComplete(title: string, body: string): Promise<void> {
  if (isMock()) return Promise.resolve();
  return invoke<void>("notify_download_complete", { title, body });
}

export function getNetworkSettings(): Promise<NetworkSettings> {
  if (isMock()) return Promise.resolve({ proxyEnabled: false, proxyUrl: "" });
  return invoke<NetworkSettings>("get_network_settings");
}

export function setNetworkSettings(settings: NetworkSettings): Promise<NetworkSettings> {
  if (isMock()) return Promise.resolve(settings);
  return invoke<NetworkSettings>("set_network_settings", { settings });
}

export function getLlmSettings(): Promise<LlmSettings> {
  if (isMock()) return Promise.resolve({ enabled: false, baseUrl: "", apiKey: "", model: "" });
  return invoke<LlmSettings>("get_llm_settings");
}

export function setLlmSettings(settings: LlmSettings): Promise<LlmSettings> {
  if (isMock()) return Promise.resolve(settings);
  return invoke<LlmSettings>("set_llm_settings", { settings });
}

export async function translateChangelog(text: string, requestId: string, onDelta: (delta: string) => void): Promise<string> {
  if (isMock()) {
    // In dev mock there is no Tauri backend; emit the whole text as one delta.
    onDelta(text);
    return text;
  }
  const unlisten = await listen<LlmTranslateDelta>("llm-translate-delta", ({ payload }) => {
    if (payload.requestId === requestId) onDelta(payload.delta);
  });
  try {
    return await invoke<string>("translate_changelog", { text, requestId });
  } finally {
    unlisten();
  }
}

export function testLlmConnection(settings: LlmSettings): Promise<string> {
  if (isMock()) return Promise.resolve("Hello");
  return invoke<string>("test_llm_connection", { settings });
}

export function getFeedStatus(): Promise<FeedStatus> {
  if (isMock()) {
    return Promise.resolve({
      configured: true,
      url: "https://null-object-0000.github.io/umanager/feed.json",
      signatureEnforced: true,
      signatureVerified: true,
      lastSuccessAtUnixSeconds: Math.floor(Date.now() / 1000) - 3600,
      generatedAtUnixSeconds: Math.floor(Date.now() / 1000) - 3600,
      applications: 5,
      developmentTools: 4,
      lastError: null,
      servingFromCache: false,
    });
  }
  return invoke<FeedStatus>("get_feed_status");
}

export function refreshFeed(): Promise<FeedStatus> {
  if (isMock()) return getFeedStatus();
  return invoke<FeedStatus>("refresh_feed");
}

export function getCategories(): Promise<CategoryCatalog | null> {
  if (isMock()) return Promise.resolve(null);
  return invoke<CategoryCatalog | null>("get_categories");
}

export function scanPackages(): Promise<ScanResult> {
  if (isMock()) {
    return Promise.resolve({
      scannedAtUnixSeconds: Math.floor(Date.now() / 1000),
      warnings: [],
      packages: [
        { packageName: "code", displayName: "Visual Studio Code", vendor: "Microsoft", installedVersion: "1.134.0-1787078834", candidateVersion: "1.134.0-1787078834", architecture: "amd64", sourceKind: "officialRepository", sourceUrl: "https://packages.microsoft.com/repos/code", updateState: "upToDate", homepage: "https://code.visualstudio.com/" },
        { packageName: "google-chrome-stable", displayName: "Google Chrome", vendor: "Google", installedVersion: "151.0.7922.169-1", candidateVersion: "151.0.7922.173-1", architecture: "amd64", sourceKind: "officialRepository", sourceUrl: "https://dl.google.com/linux/chrome-stable/deb", updateState: "updateAvailable", homepage: null },
        { packageName: "chatgpt", displayName: "ChatGPT Desktop", vendor: "OpenAI", installedVersion: "26.818.21641", candidateVersion: "26.818.41705", architecture: "amd64", sourceKind: "officialRepository", sourceUrl: "https://persistent.oaistatic.com/codex-app-prod/linux/deb", updateState: "updateAvailable", homepage: "https://developers.openai.com/codex/app" },
        { packageName: "flclash", displayName: "FlClash", vendor: "FlClash", installedVersion: "0.8.96+2026081701", candidateVersion: "0.8.97+2026082401", architecture: "amd64", sourceKind: "officialWebsite", sourceUrl: "https://github.com/chen08209/FlClash/releases/download/v0.8.97/FlClash-0.8.97-linux-amd64.deb", updateState: "updateAvailable", homepage: "https://github.com/chen08209/FlClash/releases" },
        { packageName: "wechat", displayName: "微信", vendor: "腾讯", installedVersion: "4.1.1.8", candidateVersion: "4.1.2.1", architecture: "amd64", sourceKind: "officialWebsite", sourceUrl: "https://dldir1v6.qq.com/weixin/Universal/Linux/WeChatLinux_x86_64.deb", updateState: "updateAvailable", homepage: "https://linux.weixin.qq.com/" },
        { packageName: "wemeet", displayName: "腾讯会议", vendor: "腾讯", installedVersion: "3.26.10.401", candidateVersion: null, architecture: "amd64", sourceKind: "localPackage", sourceUrl: null, updateState: "unknown", homepage: "https://meeting.tencent.com/download/" },
      ],
    });
  }
  return invoke<ScanResult>("scan_packages");
}

export function getSoftwareCatalog(): Promise<CatalogApplication[]> {
  if (isMock()) return Promise.resolve(mockCatalog);
  return invoke<CatalogApplication[]>("get_software_catalog");
}

export function getAppIcon(appId: string, iconUrl: string, iconSha256: string): Promise<string> {
  if (isMock()) return Promise.resolve("");
  return invoke<string>("fetch_app_icon", { appId, iconUrl, iconSha256 });
}

export function getApplicationDetails(applicationId: string): Promise<ApplicationDetails> {
  if (isMock()) return Promise.resolve(mockDetails(applicationId));
  return invoke<ApplicationDetails>("get_application_details", { applicationId });
}

export function getDownloadPlan(applicationId: string): Promise<DownloadPlan> {
  if (isMock()) return Promise.resolve(mockPlans[applicationId]);
  return invoke<DownloadPlan>("get_download_plan", { applicationId });
}

export async function downloadPackage(applicationId: string, packageName: string, onProgress?: (progress: DownloadProgress) => void): Promise<DownloadResult> {
  if (isMock()) {
    const plan = mockPlans[applicationId];
    onProgress?.({ packageName, phase: "downloading", transferredBytes: plan.expectedSize, totalBytes: plan.expectedSize, bytesPerSecond: 24 * 1024 * 1024 });
    onProgress?.({ packageName, phase: "verifying", transferredBytes: plan.expectedSize, totalBytes: plan.expectedSize, bytesPerSecond: 0 });
    return { plan, actualSize: plan.expectedSize, actualSha256: plan.expectedSha256 ?? "0".repeat(64), packageName: plan.packageName, version: plan.version, architecture: plan.architecture, reusedExistingFile: false, verified: true };
  }
  const unlisten = await listen<DownloadProgress>("apt-download-progress", ({ payload }) => {
    // 多个软件包可能同时下载：后端事件携带 packageName，只转发当前包的进度，避免相互串台。
    if (payload.packageName !== packageName) return;
    onProgress?.(payload);
  });
  try {
    return await invoke<DownloadResult>("download_package", { applicationId });
  } finally {
    unlisten();
  }
}

export function createOperationPlan(applicationId: string): Promise<OperationPlanArtifact> {
  return invoke<OperationPlanArtifact>("create_operation_plan", { applicationId });
}

export function launchApplication(applicationId: string): Promise<void> {
  if (isMock()) return Promise.resolve();
  return invoke<void>("launch_application", { applicationId });
}

export function openExternalUrl(url: string): Promise<void> {
  if (isMock()) return Promise.resolve();
  return invoke<void>("open_external_url", { url });
}

export function runOperationDryRun(planId: string): Promise<DryRunReport> {
  return invoke<DryRunReport>("run_operation_dry_run", { planId });
}

export function installPackage(planId: string, onProgress?: (event: OperationProgressEvent) => void): Promise<OperationExecutionReport> {
  return invokeWithOperationProgress<OperationExecutionReport>("install_package", planId, onProgress);
}

export function getInstallableApplications(): Promise<InstallableApplication[]> {
  if (isMock()) {
    const aptPlan = (applicationId: string): DownloadPlan => ({ ...mockPlans[applicationId] });
    return Promise.resolve([
      { applicationId: "vscode", packageName: "code", displayName: "Visual Studio Code", vendor: "Microsoft", homepage: "https://code.visualstudio.com/", architecture: "amd64", sourceKind: "officialRepository", installedVersion: "1.134.0-1787078834", candidateVersion: "1.134.0-1787078834", installAvailable: false, unavailableReason: "已在本机安装，请在“软件”页管理更新或卸载。", downloadPlan: null },
      { applicationId: "google-chrome", packageName: "google-chrome-stable", displayName: "Google Chrome", vendor: "Google", homepage: "https://www.google.com/chrome/", architecture: "amd64", sourceKind: "officialRepository", installedVersion: null, candidateVersion: mockPlans["google-chrome"].version, installAvailable: true, unavailableReason: null, downloadPlan: aptPlan("google-chrome") },
      { applicationId: "chatgpt", packageName: "chatgpt", displayName: "ChatGPT Desktop", vendor: "OpenAI", homepage: "https://developers.openai.com/codex/app", architecture: "amd64", sourceKind: "officialRepository", installedVersion: "26.818.21641", candidateVersion: "26.818.61809", installAvailable: false, unavailableReason: "已在本机安装，请在“软件”页管理更新或卸载。", downloadPlan: null },
      { applicationId: "wechat", packageName: "wechat", displayName: "微信", vendor: "腾讯", homepage: "https://linux.weixin.qq.com/", architecture: "amd64", sourceKind: "officialWebsite", installedVersion: "4.1.1.8", candidateVersion: "4.1.2.1", installAvailable: false, unavailableReason: "已在本机安装，请在“软件”页管理更新或卸载。", downloadPlan: null },
      { applicationId: "flclash", packageName: "flclash", displayName: "FlClash", vendor: "FlClash", homepage: "https://github.com/chen08209/FlClash/releases", architecture: "amd64", sourceKind: "officialWebsite", installedVersion: null, candidateVersion: "0.8.97+2026082401", installAvailable: true, unavailableReason: null, downloadPlan: aptPlan("flclash"), releaseNotes: "## 0.8.97\n\n- 修复托盘与代理规则导入的问题\n- 优化订阅刷新与连接稳定性\n- 升级内置 Clash 内核", releaseNotesUrl: "https://github.com/chen08209/FlClash/releases/tag/v0.8.97" },
    ]);
  }
  return invoke<InstallableApplication[]>("get_installable_applications");
}

export function getPendingLocalDeb(): Promise<LocalDebInspection | null> {
  if (isMock()) return Promise.resolve(null);
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

export function installLocalDeb(planId: string, onProgress?: (event: OperationProgressEvent) => void): Promise<OperationExecutionReport> {
  return invokeWithOperationProgress<OperationExecutionReport>("install_local_deb", planId, onProgress);
}

export function createRemovalOperationPlan(packageName: string): Promise<RemovalPlanArtifact> {
  return invoke<RemovalPlanArtifact>("create_removal_operation_plan", { packageName });
}

export function runRemovalDryRun(planId: string): Promise<RemovalExecutionReport> {
  return invoke<RemovalExecutionReport>("run_removal_dry_run", { planId });
}

export function removeManagedPackage(planId: string, onProgress?: (event: OperationProgressEvent) => void): Promise<RemovalExecutionReport> {
  return invokeWithOperationProgress<RemovalExecutionReport>("remove_managed_package", planId, onProgress);
}

async function invokeWithDevProgress<T>(command: string, toolchainId: string, version: string, onProgress?: (event: DevOperationProgress) => void): Promise<T> {
  const unlisten = onProgress ? await listen<DevOperationProgress>("dev-operation-progress", ({ payload }) => {
    if (payload.toolchainId === toolchainId) onProgress(payload);
  }) : null;
  try {
    return await invoke<T>(command, { toolchainId, version });
  } finally {
    unlisten?.();
  }
}

const mockDevToolchains: DevToolchain[] = [
  { toolchainId: "nodejs", displayName: "Node.js", vendor: "OpenJS Foundation", homepage: "https://nodejs.org/", icon: "nodejs", accentColor: "#5fa04e", manager: "nvm", managerKind: "shell", managerHome: "~/.nvm", managerScript: "nvm.sh", managerBinary: null, versionsDirectory: "~/.nvm/versions/node" },
  { toolchainId: "rust", displayName: "Rust", vendor: "Rust Project", homepage: "https://www.rust-lang.org/", icon: "rust", accentColor: "#c0562a", manager: "rustup", managerKind: "binary", managerHome: "~/.rustup", managerScript: null, managerBinary: "~/.cargo/bin/rustup", versionsDirectory: "~/.rustup/toolchains" },
];

const mockDevState: DevToolchainState = {
  toolchainId: "nodejs",
  displayName: "Node.js",
  vendor: "OpenJS Foundation",
  homepage: "https://nodejs.org/",
  manager: "nvm",
  managerFound: true,
  managerHome: "/home/user/.nvm",
  managerVersion: "0.40.6",
  defaultVersion: "v24.19.0",
  installedVersions: [
    { version: "v24.19.0", isDefault: true, isLts: true, ltsName: "krypton" },
    { version: "v22.23.2", isDefault: false, isLts: true, ltsName: "jod" },
  ],
};

const mockRustState: DevToolchainState = {
  toolchainId: "rust",
  displayName: "Rust",
  vendor: "Rust Project",
  homepage: "https://www.rust-lang.org/",
  manager: "rustup",
  managerFound: true,
  managerHome: "/home/user/.rustup",
  managerVersion: "1.29.0",
  defaultVersion: "stable",
  installedVersions: [
    { version: "stable", isDefault: true, isLts: false, ltsName: null },
  ],
};

const mockDevReleases: DevRelease[] = [
  { version: "v24.19.0", label: "LTS Krypton", recommended: true },
  { version: "v22.23.2", label: "LTS Jod", recommended: false },
  { version: "v20.20.2", label: "LTS Iron", recommended: false },
  { version: "v18.20.8", label: "LTS Hydrogen", recommended: false },
];

const mockRustReleases: DevRelease[] = [
  { version: "stable", label: "稳定版", recommended: true },
  { version: "beta", label: "测试版", recommended: false },
  { version: "nightly", label: "每日版", recommended: false },
];

export function getDevToolchains(): Promise<DevToolchain[]> {
  if (isMock()) return Promise.resolve(mockDevToolchains);
  return invoke<DevToolchain[]>("get_dev_toolchains");
}

export function getDevToolchainState(toolchainId: string): Promise<DevToolchainState> {
  if (isMock()) return Promise.resolve(toolchainId === "rust" ? mockRustState : mockDevState);
  return invoke<DevToolchainState>("get_dev_toolchain_state", { toolchainId });
}

export function getDevReleases(toolchainId: string): Promise<DevRelease[]> {
  if (isMock()) return Promise.resolve(toolchainId === "rust" ? mockRustReleases : mockDevReleases);
  return invoke<DevRelease[]>("get_dev_releases", { toolchainId });
}

export function installDevVersion(toolchainId: string, version: string, onProgress?: (event: DevOperationProgress) => void): Promise<DevOperationReport> {
  return invokeWithDevProgress<DevOperationReport>("install_dev_version", toolchainId, version, onProgress);
}

export function setDevDefaultVersion(toolchainId: string, version: string, onProgress?: (event: DevOperationProgress) => void): Promise<DevOperationReport> {
  return invokeWithDevProgress<DevOperationReport>("set_dev_default_version", toolchainId, version, onProgress);
}

export function uninstallDevVersion(toolchainId: string, version: string, onProgress?: (event: DevOperationProgress) => void): Promise<DevOperationReport> {
  return invokeWithDevProgress<DevOperationReport>("uninstall_dev_version", toolchainId, version, onProgress);
}

async function invokeWithDevToolProgress<T>(command: string, toolId: string, onProgress?: (event: DevToolProgress) => void): Promise<T> {
  const unlisten = onProgress ? await listen<DevToolProgress>("dev-tool-progress", ({ payload }) => {
    if (payload.toolId === toolId) onProgress(payload);
  }) : null;
  try {
    return await invoke<T>(command, { toolId });
  } finally {
    unlisten?.();
  }
}

const mockDevTools: DevTool[] = [
  { toolId: "claude-code", displayName: "Claude Code", vendor: "Anthropic", homepage: "https://docs.anthropic.com/en/docs/claude-code", icon: "claude", accentColor: "#b0562a", binaryName: "claude", npmPackage: "@anthropic-ai/claude-code", installer: { kind: "curlScript", scriptUrl: "https://claude.ai/install.sh", host: "claude.ai", shell: "bash" }, uninstall: { kind: "removeFiles", paths: ["~/.local/bin/claude"] }, update: { kind: "selfCommand", args: ["update"] } },
  { toolId: "opencode", displayName: "OpenCode", vendor: "OpenCode (SST)", homepage: "https://opencode.ai/", icon: "opencode", accentColor: "#d97757", binaryName: "opencode", npmPackage: "opencode-ai", installer: { kind: "curlScript", scriptUrl: "https://opencode.ai/install", host: "opencode.ai", shell: "bash" }, uninstall: { kind: "removeFiles", paths: ["~/.opencode/bin/opencode"] }, update: { kind: "selfCommand", args: ["upgrade"] } },
  { toolId: "pi", displayName: "Pi", vendor: "earendil-works", homepage: "https://pi.dev/", icon: "pi", accentColor: "#7c5ce5", binaryName: "pi", npmPackage: "@earendil-works/pi-coding-agent", installer: { kind: "curlScript", scriptUrl: "https://pi.dev/install.sh", host: "pi.dev", shell: "sh" }, uninstall: { kind: "removeFiles", paths: ["~/.local/bin/pi"] }, update: { kind: "selfCommand", args: ["update"] } },
  { toolId: "codex", displayName: "Codex CLI", vendor: "OpenAI", homepage: "https://developers.openai.com/codex/cli", icon: "codex", accentColor: "#171918", binaryName: "codex", npmPackage: "@openai/codex", installer: { kind: "npm" }, uninstall: { kind: "npm" }, update: { kind: "selfCommand", args: ["update"] } },
  { toolId: "dsh", displayName: "DeepSeek Harness", vendor: "DeepSeek", homepage: "https://github.com/deepseek-ai/deepseek-harness", icon: "dsh", accentColor: "#4D6BFE", binaryName: "dsh", npmPackage: "@deepseek-ai/dsh", installer: { kind: "npm" }, uninstall: { kind: "npm" }, update: null },
  { toolId: "hermes", displayName: "Hermes Agent", vendor: "Nous Research", homepage: "https://hermes-agent.nousresearch.com/", icon: "hermes", accentColor: "#8b5cf6", binaryName: "hermes", npmPackage: null, installer: { kind: "curlScript", scriptUrl: "https://hermes-agent.nousresearch.com/install.sh", host: "hermes-agent.nousresearch.com", shell: "bash" }, uninstall: { kind: "selfCommand", args: ["uninstall", "--yes"] }, update: { kind: "selfCommand", args: ["update"] } },
];

const mockDevToolStates: Record<string, DevToolState> = {
  "claude-code": { toolId: "claude-code", displayName: "Claude Code", vendor: "Anthropic", homepage: "https://docs.anthropic.com/en/docs/claude-code", icon: null, accentColor: "#b0562a", binaryName: "claude", npmPackage: "@anthropic-ai/claude-code", installerKind: "curlScript", npmAvailable: true, installed: true, installKind: "officialInstaller", version: "2.1.245", latestVersion: "2.1.245", binaryPath: "/home/user/.local/bin/claude", updateAvailable: false, canUninstall: true },
  opencode: { toolId: "opencode", displayName: "OpenCode", vendor: "OpenCode (SST)", homepage: "https://opencode.ai/", icon: null, accentColor: "#d97757", binaryName: "opencode", npmPackage: "opencode-ai", installerKind: "curlScript", npmAvailable: true, installed: true, installKind: "npmGlobal", version: "1.18.22", latestVersion: "1.18.22", binaryPath: "/home/user/.nvm/versions/node/v24.19.0/bin/opencode", updateAvailable: false, canUninstall: true },
  pi: { toolId: "pi", displayName: "Pi", vendor: "earendil-works", homepage: "https://pi.dev/", icon: null, accentColor: "#7c5ce5", binaryName: "pi", npmPackage: "@earendil-works/pi-coding-agent", installerKind: "curlScript", npmAvailable: true, installed: false, installKind: null, version: null, latestVersion: "0.84.3", binaryPath: null, updateAvailable: false, canUninstall: false },
  codex: { toolId: "codex", displayName: "Codex CLI", vendor: "OpenAI", homepage: "https://developers.openai.com/codex/cli", icon: null, accentColor: "#171918", binaryName: "codex", npmPackage: "@openai/codex", installerKind: "npm", npmAvailable: true, installed: true, installKind: "npmGlobal", version: "0.149.0", latestVersion: "0.149.1", binaryPath: "/home/user/.nvm/versions/node/v24.19.0/bin/codex", updateAvailable: true, canUninstall: true },
  dsh: { toolId: "dsh", displayName: "DeepSeek Harness", vendor: "DeepSeek", homepage: "https://github.com/deepseek-ai/deepseek-harness", icon: null, accentColor: "#4D6BFE", binaryName: "dsh", npmPackage: "@deepseek-ai/dsh", installerKind: "npm", npmAvailable: true, installed: true, installKind: "npmGlobal", version: "0.1.1-rc.2", latestVersion: "0.1.1-rc.2", binaryPath: "/home/user/.nvm/versions/node/v24.19.0/bin/dsh", updateAvailable: false, canUninstall: true },
  hermes: { toolId: "hermes", displayName: "Hermes Agent", vendor: "Nous Research", homepage: "https://hermes-agent.nousresearch.com/", icon: null, accentColor: "#8b5cf6", binaryName: "hermes", npmPackage: null, installerKind: "curlScript", npmAvailable: true, installed: true, installKind: "officialInstaller", version: "0.21.0", latestVersion: "0.21.0", binaryPath: "/home/user/.local/bin/hermes", updateAvailable: false, canUninstall: true },
};

export function getDevTools(): Promise<DevTool[]> {
  if (isMock()) return Promise.resolve(mockDevTools);
  return invoke<DevTool[]>("get_dev_tools");
}

export function getDevToolState(toolId: string): Promise<DevToolState> {
  if (isMock()) return Promise.resolve(mockDevToolStates[toolId] ?? mockDevToolStates["claude-code"]);
  return invoke<DevToolState>("get_dev_tool_state", { toolId });
}

export function installDevTool(toolId: string, onProgress?: (event: DevToolProgress) => void): Promise<DevToolReport> {
  return invokeWithDevToolProgress<DevToolReport>("install_dev_tool", toolId, onProgress);
}

export function updateDevTool(toolId: string, onProgress?: (event: DevToolProgress) => void): Promise<DevToolReport> {
  return invokeWithDevToolProgress<DevToolReport>("update_dev_tool", toolId, onProgress);
}

export function uninstallDevTool(toolId: string, onProgress?: (event: DevToolProgress) => void): Promise<DevToolReport> {
  return invokeWithDevToolProgress<DevToolReport>("uninstall_dev_tool", toolId, onProgress);
}

export function listScripts(): Promise<ScriptDefinition[]> {
  return invoke<ScriptDefinition[]>("list_scripts");
}

export async function runScript(scriptId: string, actionId: string, onProgress?: (event: ScriptProgressEvent) => void): Promise<ScriptRunReport> {
  const unlisten = onProgress ? await listen<ScriptProgressEvent>("script-progress", ({ payload }) => {
    if (payload.scriptId === scriptId) onProgress(payload);
  }) : null;
  try {
    return await invoke<ScriptRunReport>("run_script", { scriptId, actionId });
  } finally {
    unlisten?.();
  }
}

export function stopScript(scriptId: string): Promise<boolean> {
  return invoke<boolean>("stop_script", { scriptId });
}

const mockClipboardHistory: ClipboardEntry[] = [
  { id: 1, kind: "image", pinned: true, capturedAtMs: Date.now() - 60_000, imageWidth: 1200, imageHeight: 800, imageByteCount: 180_000, contentHash: "demo", imagePreview: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==" },
  { id: 2, kind: "text", text: "dpkg --compare-versions 2.0-1 lt 2.0-2", charCount: 40, pinned: false, capturedAtMs: Date.now() - 180_000 },
  { id: 3, kind: "text", text: "共有 3 条剪贴板记录（演示数据）", charCount: 18, pinned: false, capturedAtMs: Date.now() - 600_000 },
];

export function listClipboardHistory(): Promise<ClipboardEntry[]> {
  if (isMock()) return Promise.resolve(mockClipboardHistory);
  return invoke<ClipboardEntry[]>("list_clipboard_history");
}

export function getClipboardHistoryRevision(): Promise<number> {
  if (isMock()) return Promise.resolve(1);
  return invoke<number>("clipboard_history_revision");
}

export function copyClipboardEntry(id: number): Promise<void> {
  if (isMock()) return Promise.resolve();
  return invoke<void>("copy_clipboard_entry", { id });
}

export function getClipboardImage(id: number): Promise<string> {
  if (isMock()) return Promise.resolve(mockClipboardHistory.find((entry) => entry.id === id)?.imagePreview ?? "");
  return invoke<string>("get_clipboard_image", { id });
}

export function dragClipboardImage(id: number): Promise<void> {
  if (isMock()) return Promise.resolve();
  return invoke<void>("drag_clipboard_image", { id });
}

export function hideClipboardPanel(): Promise<void> {
  if (isMock()) return Promise.resolve();
  return invoke<void>("hide_clipboard_panel");
}

export function getClipboardHotkey(): Promise<string> {
  if (isMock()) return Promise.resolve("Super+V");
  return invoke<string>("get_clipboard_hotkey");
}

export function setClipboardHotkey(hotkey: string): Promise<string> {
  if (isMock()) return Promise.resolve(hotkey);
  return invoke<string>("set_clipboard_hotkey", { hotkey });
}

export function getSessionInfo(): Promise<SessionInfo> {
  if (isMock()) return Promise.resolve({ kind: "x11", waylandDisplay: null, display: ":0", sessionType: "x11", globalHotkeySupported: true });
  return invoke<SessionInfo>("get_session_info");
}

export function setClipboardEntryPinned(id: number, pinned: boolean): Promise<void> {
  if (isMock()) return Promise.resolve();
  return invoke<void>("set_clipboard_entry_pinned", { id, pinned });
}

export function deleteClipboardEntry(id: number): Promise<void> {
  if (isMock()) return Promise.resolve();
  return invoke<void>("delete_clipboard_entry", { id });
}

export function clearClipboardHistory(): Promise<void> {
  if (isMock()) return Promise.resolve();
  return invoke<void>("clear_clipboard_history");
}

export async function onClipboardHistoryChanged(callback: (entries: ClipboardEntry[]) => void): Promise<() => void> {
  if (isMock()) return () => {};
  return listen<ClipboardEntry[]>("clipboard-history-changed", ({ payload }) => callback(payload));
}
