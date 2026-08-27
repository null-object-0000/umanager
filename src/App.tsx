import { useEffect, useMemo, useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { createLocalDebOperationPlan, createOperationPlan, createRemovalOperationPlan, createSelfRemovalOperationPlan, createSelfUpdateOperationPlan, downloadPackage, downloadSelfUpdate, getAppIcon, getCategories, getApplicationDetails, getDevReleases, getDevToolchains, getDevToolchainState, getDevTools, getDevToolState, getDownloadPlan, getFeedStatus, getInstallableApplications, getInstallationInfo, getNetworkSettings, getPendingLocalDeb, getSelfUpdateStatus, getSoftwareCatalog, importPendingLocalDeb, installDevTool, installDevVersion, installLocalDeb, installPackage, installSelfUpdate, refreshFeed, removeManagedPackage, removeUmanager, restartApp, runLocalDebDryRun, runOperationDryRun, runRemovalDryRun, runSelfRemovalDryRun, runSelfUpdateDryRun, scanPackages, setDevDefaultVersion, setNetworkSettings, uninstallDevTool, uninstallDevVersion, listScripts, notifyDownloadComplete, runScript, stopScript } from "./api";
import type { ApplicationDetails, CatalogApplication, CategoryCatalog, DevOperationProgress, DevOperationReport, DevRelease, DevTool, DevToolchain, DevToolchainState, DevToolProgress, DevToolReport, DevToolState, DownloadPlan, DownloadProgress, DownloadResult, DryRunReport, FeedStatus, InstallableApplication, InstallationInfo, LocalDebInspection, ManagedPackage, NetworkSettings, OperationExecutionReport, OperationPlanArtifact, OperationProgressEvent, RemovalExecutionReport, RemovalPlanArtifact, ScanResult, ScriptAction, ScriptDefinition, ScriptProgressEvent, UpdateState } from "./types";
import { debCategory, devToolCategory, orderedCategories } from "./categories";
import chatgptIcon from "./assets/app-icons/chatgpt.png";
import flclashIcon from "./assets/app-icons/flclash.png";
import chromeIcon from "./assets/app-icons/google-chrome.png";
import vscodeIcon from "./assets/app-icons/vscode.png";
import wechatIcon from "./assets/app-icons/wechat.png";
import wemeetIcon from "./assets/app-icons/wemeet.svg?no-inline";
import nodejsIcon from "./assets/app-icons/nodejs.svg?no-inline";
import rustIcon from "./assets/app-icons/rust.svg?no-inline";
import claudeIcon from "./assets/app-icons/claude.svg?no-inline";
import opencodeIcon from "./assets/app-icons/opencode.svg?no-inline";
import piIcon from "./assets/app-icons/pi.svg?no-inline";
import codexIcon from "./assets/app-icons/codex.svg?no-inline";
import githubCliIcon from "./assets/app-icons/github-cli.svg?no-inline";
import feishuIcon from "./assets/app-icons/feishu.png";

type Filter = "all" | "installed" | "updates" | "installable";
type Page = "installed" | "dev" | "scripts" | "settings";
const sourceText = { officialRepository: "官方 APT 仓库", officialWebsite: "官网直连", localPackage: "本地 .deb" } as const;
const iconAssets: Record<string, string> = { vscode: vscodeIcon, "google-chrome": chromeIcon, chatgpt: chatgptIcon, flclash: flclashIcon, wechat: wechatIcon, wemeet: wemeetIcon, nodejs: nodejsIcon, rust: rustIcon, claude: claudeIcon, opencode: opencodeIcon, pi: piIcon, codex: codexIcon, "github-cli": githubCliIcon, feishu: feishuIcon };
const fallbackIconKey: Record<string, string> = { code: "vscode", "google-chrome-stable": "google-chrome", chatgpt: "chatgpt", flclash: "flclash", wechat: "wechat", wemeet: "wemeet" };
const fallbackColors: Record<string, string> = { code: "#2b78bd", "google-chrome-stable": "#4285f4", chatgpt: "#171918", flclash: "#7c5ce5", wechat: "#22ad38", wemeet: "#2878ff" };

let catalogByPackage: Record<string, CatalogApplication> = {};

function appearance(packageName: string) {
  const entry = catalogByPackage[packageName];
  return {
    iconKey: entry?.icon ?? fallbackIconKey[packageName] ?? null,
    accentColor: entry?.accentColor ?? fallbackColors[packageName] ?? "#555",
    iconUrl: entry?.iconUrl ?? null,
    iconSha256: entry?.iconSha256 ?? null,
  };
}

function Icon({ name }: { name: "apps" | "source" | "history" | "settings" | "search" | "shield" | "dev" | "script" }) {
  const paths = {
    apps: <><rect x="3" y="3" width="7" height="7" rx="2"/><rect x="14" y="3" width="7" height="7" rx="2"/><rect x="3" y="14" width="7" height="7" rx="2"/><rect x="14" y="14" width="7" height="7" rx="2"/></>,
    source: <><path d="M4 7h16M6 3h12l2 4-2 4H6L4 7l2-4Z"/><path d="M7 11v10m10-10v10M4 21h16"/></>,
    history: <><path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5M12 7v5l3 2"/></>,
    settings: <><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.9l.1.1-2.8 2.8-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.6v.2h-4V21a1.7 1.7 0 0 0-1-1.6 1.7 1.7 0 0 0-1.9.3l-.1.1L4.2 17l.1-.1a1.7 1.7 0 0 0 .3-1.9A1.7 1.7 0 0 0 3 14H2.8v-4H3a1.7 1.7 0 0 0 1.6-1 1.7 1.7 0 0 0-.3-1.9L4.2 7 7 4.2l.1.1a1.7 1.7 0 0 0 1.9.3A1.7 1.7 0 0 0 10 3V2.8h4V3a1.7 1.7 0 0 0 1 1.6 1.7 1.7 0 0 0 1.9-.3l.1-.1L19.8 7l-.1.1a1.7 1.7 0 0 0-.3 1.9 1.7 1.7 0 0 0 1.6 1h.2v4H21a1.7 1.7 0 0 0-1.6 1Z"/></>,
    search: <><circle cx="11" cy="11" r="7"/><path d="m20 20-4-4"/></>,
    shield: <><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10Z"/><path d="m9 12 2 2 4-4"/></>,
    dev: <><path d="M8 6 3 12l5 6M16 6l5 6-5 6M14 4l-4 16"/></>,
    script: <><rect x="3" y="4" width="18" height="16" rx="2"/><path d="m7 9 3 3-3 3M13 15h4"/></>,
  };
  return <svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true">{paths[name]}</svg>;
}

function AppLogo({ packageName, displayName }: { packageName: string; displayName: string }) {
  const { iconKey, accentColor, iconUrl, iconSha256 } = appearance(packageName);
  const asset = iconKey ? iconAssets[iconKey] : undefined;
  const [remoteSrc, setRemoteSrc] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    setRemoteSrc(null);
    if (!asset && iconUrl && iconSha256) {
      getAppIcon(packageName, iconUrl, iconSha256)
        .then((dataUrl) => { if (!cancelled && dataUrl) setRemoteSrc(dataUrl); })
        .catch(() => { /* 拉取失败保持字母头像兜底 */ });
    }
    return () => { cancelled = true; };
  }, [packageName, asset, iconUrl, iconSha256]);
  const src = asset ?? remoteSrc;
  const letter = displayName === "微信" ? "微" : displayName === "腾讯会议" ? "会" : displayName.slice(0, 1).toUpperCase();
  return <span className={`app-mark ${src ? "has-icon" : ""}`} style={{ background: src ? undefined : accentColor }}>{src ? <img src={src} alt=""/> : letter}</span>;
}

function AppMark({ item }: { item: ManagedPackage }) {
  return <AppLogo packageName={item.packageName} displayName={item.displayName}/>;
}

function DevLogo({ toolchain }: { toolchain: DevToolchain }) {
  const asset = toolchain.icon ? iconAssets[toolchain.icon] : undefined;
  if (asset) {
    return <span className="app-mark has-icon"><img src={asset} alt=""/></span>;
  }
  return <span className="app-mark" style={{ background: toolchain.accentColor ?? "#555" }}>{toolchain.displayName.slice(0, 1).toUpperCase()}</span>;
}

function DevToolLogo({ tool }: { tool: DevTool }) {
  const asset = tool.icon ? iconAssets[tool.icon] : undefined;
  if (asset) {
    return <span className="app-mark has-icon"><img src={asset} alt=""/></span>;
  }
  return <span className="app-mark" style={{ background: tool.accentColor ?? "#555" }}>{tool.displayName.slice(0, 1).toUpperCase()}</span>;
}

function isAutoInstallable(packageName: string) {
  const entry = catalogByPackage[packageName];
  if (!entry) return false;
  return ["aptRepository", "stableDownloadEndpoint", "releaseApi", "versionEndpoint"].includes(entry.source.kind);
}

function formatBytes(bytes: number) {
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MiB`;
}

function formatSpeed(bytesPerSecond: number) {
  if (bytesPerSecond >= 1024 * 1024) return `${(bytesPerSecond / 1024 / 1024).toFixed(1)} MiB/s`;
  return `${(bytesPerSecond / 1024).toFixed(0)} KiB/s`;
}

const versionSourceLabel: Record<string, string> = {
  official: "官方发布时间",
  serverModified: "官方安装包更新时间",
  observed: "首次采集到该版本",
};

function formatUpdatedAt(unixSeconds: number | null | undefined) {
  if (unixSeconds == null) return null;
  return new Date(unixSeconds * 1000).toLocaleString("zh-CN", { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" });
}

function UpdatedAtLine({ seconds, source }: { seconds: number | null | undefined; source: string | null | undefined }) {
  const text = formatUpdatedAt(seconds);
  if (!text) return null;
  const label = source ? (versionSourceLabel[source] ?? "") : "";
  return <span className="updated-at-line">{label ? `${label}：` : ""}{text}</span>;
}

function OperationLogPanel({ events, running }: { events: OperationProgressEvent[]; running: boolean }) {
  if (events.length === 0) return null;
  const phases = events.filter((event) => event.kind !== "log");
  const logs = events.filter((event) => event.kind === "log");
  return <section className="operation-progress-panel" aria-live="polite">
    <header><div><span className={running ? "operation-pulse" : "operation-complete-mark"}>{running ? "" : "✓"}</span><div><strong>{running ? "正在执行系统包操作" : "系统包操作已结束"}</strong><span>只读输出，不能输入或执行命令</span></div></div></header>
    <div className="operation-phase-list">{phases.map((event, index) => <div className={event.kind} key={`${index}-${event.message}`}><span>{event.kind === "completed" ? "✓" : event.kind === "warning" ? "!" : "•"}</span><p>{event.message}</p></div>)}</div>
    <details className="operation-log-details" open={running || undefined}>
      <summary>详细日志 <b>{logs.length}</b></summary>
      <div className="operation-terminal" role="log">{logs.length === 0 ? <span className="terminal-placeholder">等待 dpkg 输出…</span> : logs.map((event, index) => <div className={event.stream} key={`${index}-${event.message}`}><span>{event.stream === "stderr" ? "ERR" : "OUT"}</span><code>{event.message}</code></div>)}</div>
    </details>
  </section>;
}

function appendProgress(setter: Dispatch<SetStateAction<OperationProgressEvent[]>>, event: OperationProgressEvent) {
  setter((current) => [...current.slice(-499), event]);
}

function LocalDebDialog({ initial, onClose, onInstalled }: { initial: LocalDebInspection; onClose: () => void; onInstalled: () => void }) {
  const [inspected, setInspected] = useState(initial);
  const [confirmed, setConfirmed] = useState(false);
  const [plan, setPlan] = useState<OperationPlanArtifact | null>(null);
  const [dryRun, setDryRun] = useState<OperationExecutionReport | null>(null);
  const [installed, setInstalled] = useState<OperationExecutionReport | null>(null);
  const [busy, setBusy] = useState<"import" | "plan" | "dry-run" | "install" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [progressEvents, setProgressEvents] = useState<OperationProgressEvent[]>([]);
  const disposition = { newInstall: "新安装", upgrade: "升级", reinstall: "同版本重装", downgrade: "降级", unsupportedArchitecture: "架构不兼容" }[inspected.disposition];
  const run = async <T,>(kind: typeof busy, action: () => Promise<T>, complete: (value: T) => void) => {
    setBusy(kind); setError(null);
    try { complete(await action()); } catch (reason) { setError(String(reason)); } finally { setBusy(null); }
  };
  return <div className="local-deb-layer">
    <section className="local-deb-dialog" role="dialog" aria-modal="true" aria-label="安装本地 Debian 包">
      <header><div><span className="deb-file-mark">DEB</span><div><h2>安装本地软件包</h2><p>{inspected.fileName}</p></div></div><button className="close-button" onClick={onClose} aria-label="关闭">×</button></header>
      <div className="local-deb-content">
        <div className="untrusted-banner"><strong>⚠ 来源未验证</strong><span>这是用户提供的本地文件。UManager 可验证文件一致性，但无法证明它由官方厂商发布。</span></div>
        <dl className="local-deb-facts">
          <div><dt>包名</dt><dd>{inspected.packageName}</dd></div><div><dt>动作</dt><dd>{disposition}</dd></div>
          <div><dt>已安装</dt><dd>{inspected.installedVersion ?? "未安装"}</dd></div><div><dt>目标版本</dt><dd>{inspected.version}</dd></div>
          <div><dt>架构</dt><dd>{inspected.architecture}</dd></div><div><dt>大小</dt><dd>{formatBytes(inspected.size)}</dd></div>
          <div className="wide"><dt>SHA-256</dt><dd title={inspected.sha256}>{inspected.sha256}</dd></div><div className="wide"><dt>原始路径</dt><dd title={inspected.originalPath}>{inspected.originalPath}</dd></div>
        </dl>
        {!inspected.installAllowed && <div className="blocked-install">UManager 拒绝同版本重装、降级或不兼容架构的安装包。</div>}
        {error && <div className="inline-error">{error}</div>}
        {inspected.installAllowed && !inspected.cachedPath && <button className="download-button" disabled={busy !== null} onClick={() => void run("import", importPendingLocalDeb, setInspected)}>{busy === "import" ? "正在校验并导入…" : "校验并导入 UManager 缓存"}</button>}
        {inspected.cachedPath && !plan && <><label className="plan-confirmation"><input type="checkbox" checked={confirmed} onChange={(event) => setConfirmed(event.target.checked)}/><span>我信任该文件来源，已核对包名、版本、架构和 SHA-256，并理解 `.deb` 安装脚本将以 root 权限运行。</span></label><button className="download-button" disabled={!confirmed || busy !== null} onClick={() => void run("plan", () => createLocalDebOperationPlan(inspected.sha256), setPlan)}>{busy === "plan" ? "正在锁定计划…" : "确认并锁定安装计划"}</button></>}
        {plan && <div className="immutable-plan"><strong>计划已锁定</strong><span>ID：{plan.plan.planId}</span><span>有效至：{new Date(plan.plan.payload.expiresAtUnixSeconds * 1000).toLocaleTimeString("zh-CN")}</span></div>}
        {plan && !dryRun && <button className="dry-run-button" disabled={busy !== null} onClick={() => void run("dry-run", () => runLocalDebDryRun(plan!.plan.planId), setDryRun)}>{busy === "dry-run" ? "正在特权环境复核…" : "授权并执行安装前 dry-run"}</button>}
        {plan && dryRun && !installed && <><div className="dry-run-success"><strong>✓ 安装前复核通过</strong><span>helper 已独立复核路径、包名、版本、架构、大小和 SHA-256。</span></div><button className="install-local-button" disabled={busy !== null} onClick={() => void run("install", () => { setProgressEvents([]); return installLocalDeb(plan!.plan.planId, (event) => appendProgress(setProgressEvents, event)); }, (value) => { setInstalled(value); onInstalled(); })}>{busy === "install" ? "正在安装…" : `授权并安装 ${inspected.packageName}`}</button></>}
        <OperationLogPanel events={progressEvents} running={busy === "install"}/>
        {installed && <div className="dry-run-success"><strong>✓ 安装命令已成功完成</strong><span>已重新扫描；若该软件已有 UManager 厂商适配，它会出现在受管列表中。</span></div>}
      </div>
    </section>
  </div>;
}

function RemovalDialog({ item, onClose, onRemoved }: { item: ManagedPackage; onClose: () => void; onRemoved: () => void }) {
  const [confirmed, setConfirmed] = useState(false);
  const [plan, setPlan] = useState<RemovalPlanArtifact | null>(null);
  const [dryRun, setDryRun] = useState<RemovalExecutionReport | null>(null);
  const [removed, setRemoved] = useState<RemovalExecutionReport | null>(null);
  const [busy, setBusy] = useState<"plan" | "dry-run" | "remove" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [progressEvents, setProgressEvents] = useState<OperationProgressEvent[]>([]);
  const run = async <T,>(kind: typeof busy, action: () => Promise<T>, complete: (value: T) => void) => {
    setBusy(kind); setError(null);
    try { complete(await action()); } catch (reason) { setError(String(reason)); } finally { setBusy(null); }
  };

  return <div className="local-deb-layer">
    <section className="local-deb-dialog removal-dialog" role="dialog" aria-modal="true" aria-label={`卸载 ${item.displayName}`}>
      <header><div><AppMark item={item}/><div><h2>卸载 {item.displayName}</h2><p>{item.vendor} · {item.packageName}</p></div></div><button className="close-button" onClick={onClose} disabled={busy !== null} aria-label="关闭">×</button></header>
      <div className="local-deb-content">
        <div className="removal-warning"><strong>这会从系统中移除该软件</strong><span>UManager 不会请求 purge、自动删除依赖或直接删除你的个人目录；但 Debian 包自带的卸载脚本仍会以 root 权限运行。</span></div>
        <dl className="local-deb-facts">
          <div><dt>包名</dt><dd>{item.packageName}</dd></div><div><dt>动作</dt><dd>dpkg --remove</dd></div>
          <div><dt>已安装</dt><dd>{item.installedVersion}</dd></div><div><dt>架构</dt><dd>{item.architecture}</dd></div>
          <div className="wide"><dt>范围</dt><dd>仅移除白名单中的 {item.packageName} 包</dd></div>
        </dl>
        {error && <div className="inline-error removal-error">{error}</div>}
        {!plan && <><label className="plan-confirmation removal-confirmation"><input type="checkbox" checked={confirmed} onChange={(event) => setConfirmed(event.target.checked)}/><span>我已核对软件、包名、版本和架构，并理解卸载脚本将以 root 权限运行。</span></label><button className="download-button" disabled={!confirmed || busy !== null} onClick={() => void run("plan", () => createRemovalOperationPlan(item.packageName), setPlan)}>{busy === "plan" ? "正在锁定计划…" : "确认并锁定卸载计划"}</button></>}
        {plan && <div className="immutable-plan"><strong>卸载计划已锁定</strong><span>ID：{plan.plan.planId}</span><span>有效至：{new Date(plan.plan.payload.expiresAtUnixSeconds * 1000).toLocaleTimeString("zh-CN")}</span></div>}
        {plan && !dryRun && <button className="dry-run-button" disabled={busy !== null} onClick={() => void run("dry-run", () => runRemovalDryRun(plan.plan.planId), setDryRun)}>{busy === "dry-run" ? "正在特权环境复核…" : "授权并执行卸载前复核"}</button>}
        {dryRun && !removed && <><div className="dry-run-success"><strong>✓ 卸载前复核通过</strong><span>helper 已重新核对白名单、当前版本、架构和不可变计划；本次未修改系统。</span></div><button className="remove-confirm-button" disabled={busy !== null} onClick={() => void run("remove", () => { setProgressEvents([]); return removeManagedPackage(plan!.plan.planId, (event) => appendProgress(setProgressEvents, event)); }, (value) => { setRemoved(value); onRemoved(); })}>{busy === "remove" ? "正在卸载…" : `再次确认并卸载 ${item.displayName}`}</button></>}
        <OperationLogPanel events={progressEvents} running={busy === "remove"}/>
        {removed && <><div className="dry-run-success"><strong>✓ 卸载已完成</strong><span>dpkg 移除命令已成功结束，软件列表已重新扫描。</span></div><button className="download-button" onClick={onClose}>完成</button></>}
      </div>
    </section>
  </div>;
}

function SelfRemovalDialog({ info, onClose }: { info: InstallationInfo; onClose: () => void }) {
  const [confirmed, setConfirmed] = useState(false);
  const [plan, setPlan] = useState<RemovalPlanArtifact | null>(null);
  const [dryRun, setDryRun] = useState<RemovalExecutionReport | null>(null);
  const [removed, setRemoved] = useState<RemovalExecutionReport | null>(null);
  const [busy, setBusy] = useState<"plan" | "dry-run" | "remove" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [progressEvents, setProgressEvents] = useState<OperationProgressEvent[]>([]);
  const run = async <T,>(kind: typeof busy, action: () => Promise<T>, complete: (value: T) => void) => {
    setBusy(kind); setError(null);
    try { complete(await action()); } catch (reason) { setError(String(reason)); } finally { setBusy(null); }
  };

  return <div className="local-deb-layer">
    <section className="local-deb-dialog removal-dialog self-removal-dialog" role="dialog" aria-modal="true" aria-label="卸载 UManager">
      <header><div><span className="brand-mark">U</span><div><h2>卸载 UManager</h2><p>{info.packageName} · {info.packageVersion} · {info.architecture}</p></div></div><button className="close-button" onClick={onClose} disabled={busy !== null} aria-label="关闭">×</button></header>
      <div className="local-deb-content">
        <div className="removal-warning"><strong>这会移除 UManager 程序本身</strong><span>只执行固定的 <code>/usr/bin/dpkg --remove u-manager</code>。不会请求 purge，也不会删除 UManager 缓存、下载的软件包、操作计划或你的个人文件。</span></div>
        <dl className="local-deb-facts">
          <div><dt>包名</dt><dd>{info.packageName}</dd></div><div><dt>动作</dt><dd>remove-umanager</dd></div>
          <div><dt>已安装</dt><dd>{info.packageVersion}</dd></div><div><dt>架构</dt><dd>{info.architecture}</dd></div>
          <div className="wide"><dt>执行后</dt><dd>当前窗口可继续显示结果；关闭后 UManager 将无法再次启动</dd></div>
        </dl>
        {error && <div className="inline-error removal-error">{error}</div>}
        {!plan && <><label className="plan-confirmation removal-confirmation"><input type="checkbox" checked={confirmed} onChange={(event) => setConfirmed(event.target.checked)}/><span>我确认要卸载当前 `.deb` 安装的 UManager，并理解保留的缓存数据需要日后手动清理。</span></label><button className="download-button" disabled={!confirmed || busy !== null} onClick={() => void run("plan", createSelfRemovalOperationPlan, setPlan)}>{busy === "plan" ? "正在锁定计划…" : "确认并锁定自卸载计划"}</button></>}
        {plan && <div className="immutable-plan"><strong>自卸载计划已锁定</strong><span>ID：{plan.plan.planId}</span><span>有效至：{new Date(plan.plan.payload.expiresAtUnixSeconds * 1000).toLocaleTimeString("zh-CN")}</span></div>}
        {plan && !dryRun && <button className="dry-run-button" disabled={busy !== null} onClick={() => void run("dry-run", () => runSelfRemovalDryRun(plan.plan.planId), setDryRun)}>{busy === "dry-run" ? "正在特权环境复核…" : "授权并执行卸载前复核"}</button>}
        {dryRun && !removed && <><div className="dry-run-success"><strong>✓ 自卸载前复核通过</strong><span>helper 已重新核对动作、固定包名、版本、架构和不可变计划；本次未修改系统。</span></div><button className="remove-confirm-button" disabled={busy !== null} onClick={() => void run("remove", () => { setProgressEvents([]); return removeUmanager(plan!.plan.planId, (event) => appendProgress(setProgressEvents, event)); }, setRemoved)}>{busy === "remove" ? "正在卸载 UManager…" : "再次确认并卸载 UManager"}</button></>}
        <OperationLogPanel events={progressEvents} running={busy === "remove"}/>
        {removed && <div className="dry-run-success"><strong>✓ UManager 已从系统移除</strong><span>关闭当前窗口后程序将退出；缓存和个人数据仍然保留。</span></div>}
      </div>
    </section>
  </div>;
}

function SelfUpdateDialog({ info, onClose, onUpdated }: { info: InstallationInfo; onClose: () => void; onUpdated: () => void }) {
  const [status, setStatus] = useState<ApplicationDetails | null>(null);
  const [downloaded, setDownloaded] = useState<DownloadResult | null>(null);
  const [downloadProgress, setDownloadProgress] = useState<DownloadProgress | null>(null);
  const [plan, setPlan] = useState<OperationPlanArtifact | null>(null);
  const [dryRun, setDryRun] = useState<OperationExecutionReport | null>(null);
  const [installed, setInstalled] = useState<OperationExecutionReport | null>(null);
  const [busy, setBusy] = useState<"check" | null>(null);
  const [phase, setPhase] = useState<"idle" | "downloading" | "ready" | "planning" | "dry-run" | "installing" | "done">("idle");
  const [error, setError] = useState<string | null>(null);
  const [progressEvents, setProgressEvents] = useState<OperationProgressEvent[]>([]);

  const check = () => {
    setDownloaded(null); setDownloadProgress(null); setPlan(null); setDryRun(null); setInstalled(null); setPhase("idle"); setProgressEvents([]);
    setBusy("check"); setError(null);
    getSelfUpdateStatus().then(setStatus).catch((reason) => setError(String(reason))).finally(() => setBusy(null));
  };
  useEffect(() => { void check(); }, []);

  const updateAvailable = status?.updateState === "updateAvailable";
  const running = phase === "downloading" || phase === "planning" || phase === "dry-run" || phase === "installing";

  const start = async () => {
    setError(null);
    setDownloadProgress(null); setDownloaded(null); setPlan(null); setDryRun(null); setInstalled(null); setProgressEvents([]);
    try {
      setPhase("downloading");
      const download = await downloadSelfUpdate(setDownloadProgress);
      setDownloaded(download);
      if (!download.verified) throw new Error("更新包校验未通过，已停止，未更改系统。");
      setPhase("ready");
      if (!document.hasFocus()) {
        void notifyDownloadComplete("UManager 更新包下载完成", "安装包已通过校验，回到 UManager 继续更新（需系统授权）。").catch(() => { /* 通知失败不影响流程 */ });
      }
    } catch (reason) {
      setError(String(reason));
      setPhase("idle");
    }
  };

  const install = async () => {
    setError(null);
    try {
      setPhase("planning");
      const lockedPlan = await createSelfUpdateOperationPlan();
      setPlan(lockedPlan);
      setPhase("dry-run");
      const dry = await runSelfUpdateDryRun(lockedPlan.plan.planId);
      setDryRun(dry);
      setPhase("installing");
      const report = await installSelfUpdate(lockedPlan.plan.planId, (event) => appendProgress(setProgressEvents, event));
      setInstalled(report);
      onUpdated();
      setPhase("done");
    } catch (reason) {
      setError(String(reason));
      setPhase("ready");
    }
  };

  return <div className="local-deb-layer">
    <section className="local-deb-dialog removal-dialog self-update-dialog" role="dialog" aria-modal="true" aria-label="更新 UManager">
      <header><div><span className="brand-mark">U</span><div><h2>更新 UManager</h2><p>{info.packageName} · {info.packageVersion}{status?.candidateVersion ? ` → ${status.candidateVersion}` : ""}</p></div></div><button className="close-button" onClick={onClose} disabled={busy !== null || running} aria-label="关闭">×</button></header>
      <div className="local-deb-content">
        <div className="removal-warning"><strong>自更新会用新 `.deb` 替换当前程序</strong><span>安装包来自 UManager 官方 GitHub Release；SHA-256 与发布摘要一致后，再确认才会生成不可变计划、特权复核并安装。</span></div>
        {error && <div className="inline-error removal-error">{error}</div>}
        {busy === "check" && !status && <div className="settings-loading"><span className="loader"/><span>正在读取最新发布…</span></div>}
        {status && <dl className="local-deb-facts">
          <div><dt>当前版本</dt><dd>{status.installedVersion ?? "未安装"}</dd></div><div><dt>最新版本</dt><dd>{status.candidateVersion ?? "—"}</dd></div>
          <div><dt>发布标签</dt><dd>{status.releaseTag ?? "—"}</dd></div><div><dt>资产</dt><dd>{status.assetName ?? "—"}</dd></div>
          <div><dt>大小</dt><dd>{status.expectedSize != null ? formatBytes(status.expectedSize) : "—"}</dd></div>
          <div className="wide"><dt>SHA-256</dt><dd title={status.sha256 ?? undefined}>{status.sha256 ?? "—"}</dd></div>
        </dl>}
        {status && !updateAvailable && <div className="dry-run-success"><strong>✓ 已是最新版本</strong><span>当前 `.deb` 安装的 UManager 与最新发布一致，无需更新。</span></div>}
        {status && updateAvailable && <>
          {phase === "idle" && <button className="download-button" disabled={busy !== null} onClick={() => void start()}>更新到 {status.candidateVersion ?? "最新版本"}</button>}
          {phase === "ready" && <button className="download-button" disabled={busy !== null} onClick={() => void install()}>继续更新（需系统授权）</button>}
          {phase !== "idle" && phase !== "ready" && phase !== "done" && <button className="download-button" disabled>{phase === "downloading" ? "正在下载并校验…" : phase === "planning" ? "正在生成不可变计划…" : phase === "dry-run" ? "正在特权环境复核…" : "正在安装更新…"}</button>}
          <button className="secondary-button" style={{ marginTop: 10 }} disabled={busy !== null || running} onClick={check}>重新检查</button>
        </>}
        {phase === "downloading" && downloadProgress && <DownloadProgressCard progress={downloadProgress} displayName="UManager"/>}
        {downloaded?.verified && <div className="dry-run-success"><strong>✓ 下载校验通过</strong><span>版本 {downloaded.version} · SHA-256 与 GitHub 发布摘要一致。</span></div>}
        {plan && <div className="immutable-plan"><strong>自更新计划已锁定</strong><span>ID：{plan.plan.planId}</span><span>有效至：{new Date(plan.plan.payload.expiresAtUnixSeconds * 1000).toLocaleTimeString("zh-CN")}</span></div>}
        {dryRun && <div className="dry-run-success"><strong>✓ 自更新前复核通过</strong><span>helper 已重新核对动作、固定包名、版本、架构和不可变计划；本次未修改系统。</span></div>}
        <OperationLogPanel events={progressEvents} running={phase === "installing"}/>
        {installed && <>
          <div className="dry-run-success"><strong>✓ 更新命令已成功完成</strong><span>新的 UManager 已通过 dpkg 安装。</span></div>
          <button className="download-button" onClick={() => { void restartApp().catch((reason) => setError(String(reason))); }}>重启 UManager</button>
        </>}
      </div>
    </section>
  </div>;
}

function NetworkSettingsPanel() {
  const [settings, setSettings] = useState<NetworkSettings | null>(null);
  const [enabled, setEnabled] = useState(false);
  const [proxyUrl, setProxyUrl] = useState("");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setLoading(true); setError(null);
    getNetworkSettings()
      .then((value) => {
        setSettings(value);
        setEnabled(value.proxyEnabled);
        setProxyUrl(value.proxyUrl);
      })
      .catch((reason) => setError(String(reason)))
      .finally(() => setLoading(false));
  }, []);

  const save = async () => {
    setSaving(true); setError(null); setSaved(false);
    try {
      const value = await setNetworkSettings({ proxyEnabled: enabled, proxyUrl });
      setSettings(value);
      setEnabled(value.proxyEnabled);
      setProxyUrl(value.proxyUrl);
      setSaved(true);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSaving(false);
    }
  };

  return <section className="settings-panel network-panel">
    <div className="settings-section-heading"><div><div><h2>网络代理</h2><p>让官网更新检查与安装包下载经本地代理访问网络</p></div></div><span className={`install-kind-badge ${enabled ? "" : "unknown"}`}>{enabled ? "已启用" : "未启用"}</span></div>
    {loading && <div className="settings-loading"><span className="loader"/><span>正在读取网络代理设置…</span></div>}
    {!loading && <>
      <div className="network-proxy-form">
        <label className="plan-confirmation proxy-toggle"><input type="checkbox" checked={enabled} onChange={(event) => { setEnabled(event.target.checked); setSaved(false); }} /><span>启用代理（如 FlClash 的本地代理服务）</span></label>
        <label className="proxy-url-field"><span>代理地址</span><input value={proxyUrl} onChange={(event) => { setProxyUrl(event.target.value); setSaved(false); }} placeholder="http://127.0.0.1:7890" disabled={!enabled} spellCheck={false} aria-label="代理地址"/></label>
        <p className="proxy-hint">FlClash 等本地代理通常监听 <code>http://127.0.0.1:7890</code>，代理地址支持 <code>http://</code>、<code>https://</code>、<code>socks5://</code> 或 <code>socks5h://</code>。</p>
        {error && <div className="inline-error">{error}</div>}
        <div className="proxy-actions"><button className="primary-button" onClick={() => void save()} disabled={saving}>{saving ? "保存中…" : "保存代理设置"}</button>{saved && !error && <span className="proxy-saved">✓ 已保存，后续检查更新与下载将使用该代理</span>}</div>
      </div>
      <dl className="installation-facts network-facts">
        <div><dt>当前状态</dt><dd>{enabled ? "代理已启用" : "未使用代理"}</dd></div>
        <div><dt>代理地址</dt><dd title={settings?.proxyUrl ?? ""}>{settings?.proxyUrl && settings.proxyUrl.trim() ? settings.proxyUrl : "未设置"}</dd></div>
      </dl>
    </>}
  </section>;
}

function FeedStatusPanel() {
  const [status, setStatus] = useState<FeedStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = () => {
    setLoading(true); setError(null);
    getFeedStatus()
      .then((value) => setStatus(value))
      .catch((reason) => setError(String(reason)))
      .finally(() => setLoading(false));
  };

  useEffect(() => { load(); }, []);

  const refresh = () => {
    setRefreshing(true); setError(null);
    refreshFeed()
      .then((value) => setStatus(value))
      .catch((reason) => setError(String(reason)))
      .finally(() => setRefreshing(false));
  };

  const healthy = status?.configured && !status.lastError;
  const servingCache = status?.servingFromCache;
  const formatTime = (seconds: number | null) => seconds
    ? new Date(seconds * 1000).toLocaleString("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" })
    : "—";

  return <section className="settings-panel network-panel">
    <div className="settings-section-heading">
      <div><div><h2>软件信息源</h2><p>候选版本、大小与 SHA-256 来自 UManager 官方采集镜像（GitHub Actions → Pages）</p></div></div>
      <div>
        <span className={`install-kind-badge ${healthy ? "" : "unknown"}`}>{healthy ? "已连接" : (status?.configured ? "不可用" : "未配置")}</span>
        <button className="secondary-button" onClick={refresh} disabled={refreshing || loading}>{refreshing ? "刷新中…" : "立即刷新"}</button>
      </div>
    </div>
    {loading && <div className="settings-loading"><span className="loader"/><span>正在读取软件信息源状态…</span></div>}
    {!loading && <>
      {servingCache && <div className="inline-note">当前展示本地缓存（上次成功获取 {formatTime(status?.lastSuccessAtUnixSeconds ?? null)}），后台正在尝试更新。</div>}
      {status?.lastError && <div className="inline-error">{servingCache ? `${status.lastError}（仍展示本地缓存）` : status.lastError}</div>}
      {error && <div className="inline-error">{error}</div>}
      <dl className="installation-facts network-facts">
        <div><dt>地址</dt><dd title={status?.url ?? ""}>{status?.url ?? "未配置"}</dd></div>
        <div><dt>数据来源</dt><dd>{servingCache ? "本地缓存" : (status?.signatureVerified ? "本次联网获取" : "—")}</dd></div>
        <div><dt>最近成功</dt><dd>{formatTime(status?.lastSuccessAtUnixSeconds ?? null)}</dd></div>
        <div><dt>抓取时间</dt><dd>{formatTime(status?.generatedAtUnixSeconds ?? null)}</dd></div>
        <div><dt>覆盖</dt><dd>{status ? `${status.applications} 个应用 · ${status.developmentTools} 个开发工具` : "—"}</dd></div>
        <div><dt>签名</dt><dd>{status?.signatureVerified ? "Ed25519 已校验" : (status?.signatureEnforced ? "校验失败" : "未启用")}</dd></div>
      </dl>
    </>}
  </section>;
}

function SettingsPage({ info, loading, error, onRefresh, onRemove, onUpdate }: { info: InstallationInfo | null; loading: boolean; error: string | null; onRefresh: () => void; onRemove: () => void; onUpdate: () => void }) {
  const kindLabel = info ? { debianPackage: ".deb 安装版", portable: "便携版", development: "开发版" }[info.installationKind] : "检测中";
  const kindDescription = info?.installationKind === "debianPackage"
    ? "当前可执行文件属于系统中已安装的 Debian 软件包，可通过 UManager 安全卸载。"
    : info?.installationKind === "development"
      ? "当前从开发构建目录运行，不属于系统软件包；请直接停止开发进程。"
      : "当前可执行文件不属于已安装的 Debian 软件包；退出后可直接删除文件。";
  return <main className="workspace settings-workspace">
    <header className="workspace-header"><div><h1>设置</h1><p>查看 UManager 版本、安装形态与维护选项</p></div><button className="secondary-button" onClick={onRefresh} disabled={loading}>{loading ? "检测中…" : "重新检测"}</button></header>
    <section className="settings-panel">
      <div className="settings-section-heading"><div><span className="brand-mark large">U</span><div><h2>UManager</h2><p>Ubuntu 个人软件管家</p></div></div><span className={`install-kind-badge ${info?.installationKind ?? "unknown"}`}>{kindLabel}</span></div>
      {error && <div className="message error"><strong>无法检测安装形态</strong><span>{error}</span></div>}
      {loading && !info && <div className="settings-loading"><span className="loader"/><span>正在核对可执行文件与 dpkg 安装清单…</span></div>}
      {info && <>
        <dl className="installation-facts">
          <div><dt>应用版本</dt><dd>{info.appVersion}</dd></div>
          <div><dt>运行形态</dt><dd>{kindLabel}</dd></div>
          <div><dt>Debian 包</dt><dd>{info.packageName ? `${info.packageName} ${info.packageVersion}` : "不属于已安装软件包"}</dd></div>
          <div><dt>架构</dt><dd>{info.architecture ?? "跟随当前构建"}</dd></div>
          <div className="wide"><dt>运行位置</dt><dd title={info.executablePath}>{info.executablePath}</dd></div>
        </dl>
        <p className="installation-description"><Icon name="shield"/>{kindDescription}</p>
        <div className="self-update-zone"><div><strong>更新 UManager</strong><span>{info.canSelfRemove ? "从官方 GitHub Release 检查并安装新版本；下载、SHA-256 校验、不可变计划与双重授权。仅 `.deb` 安装版可用。" : "便携版 / 开发版不通过 dpkg 更新；请重新下载或重新构建。"}</span></div><button className="self-update-button" disabled={!info.canSelfRemove} onClick={onUpdate}>检查更新</button></div>
        <div className="danger-zone"><div><strong>卸载 UManager</strong><span>{info.canSelfRemove ? "移除程序本身，保留缓存和个人数据。操作前会展示不可变计划并请求系统授权。" : "此运行形态不能通过 dpkg 卸载。"}</span></div><button className="self-remove-button" disabled={!info.canSelfRemove} onClick={onRemove}>卸载 UManager</button></div>
      </>}
    </section>
    <NetworkSettingsPanel/>
    <FeedStatusPanel/>
  </main>;
}

function DownloadCard({ plan }: { plan: DownloadPlan }) {
  const isWebsite = plan.sourceKind === "officialWebsite";
  return <div className="download-plan-card"><div className="download-file"><div><strong>{plan.fileName}</strong><span>{formatBytes(plan.expectedSize)} · {plan.architecture}</span></div><span className="apt-index-badge">{isWebsite ? "发布资产" : "APT 索引"}</span></div><dl><div><dt>版本</dt><dd>{plan.version}</dd></div><div><dt>SHA-256</dt><dd title={plan.expectedSha256 ?? undefined}>{plan.expectedSha256 ?? "下载后计算"}</dd></div><div><dt>缓存位置</dt><dd title={plan.targetPath}>{plan.targetPath}</dd></div></dl></div>;
}

function DownloadProgressCard({ progress, displayName }: { progress: DownloadProgress; displayName: string }) {
  const percent = Math.min(100, Math.round(progress.transferredBytes / progress.totalBytes * 100));
  return <div className="download-progress-card" aria-live="polite">
    <div className="download-progress-title"><strong>{progress.phase === "downloading" ? `正在下载 ${displayName}` : `正在校验 ${displayName} 安装包`}</strong><span>{percent}%</span></div>
    <div className="download-progress-track"><span style={{ width: `${percent}%` }}/></div>
    <div className="download-progress-stats"><span>{formatBytes(progress.transferredBytes)} / {formatBytes(progress.totalBytes)}</span><strong>{progress.phase === "downloading" ? formatSpeed(progress.bytesPerSecond) : "正在复核包信息与 SHA-256"}</strong></div>
  </div>;
}

function UpdateDrawer({ item, onClose, onInstalled }: { item: ManagedPackage; onClose: () => void; onInstalled: () => void }) {
  const applicationId = catalogByPackage[item.packageName]?.applicationId;
  const [details, setDetails] = useState<ApplicationDetails | null>(null);
  const [downloadPlan, setDownloadPlan] = useState<DownloadPlan | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [downloadResult, setDownloadResult] = useState<DownloadResult | null>(null);
  const [downloadProgress, setDownloadProgress] = useState<DownloadProgress | null>(null);
  const [operationPlan, setOperationPlan] = useState<OperationPlanArtifact | null>(null);
  const [dryRun, setDryRun] = useState<OperationExecutionReport | null>(null);
  const [installed, setInstalled] = useState<OperationExecutionReport | null>(null);
  const [phase, setPhase] = useState<"idle" | "downloading" | "ready" | "planning" | "dry-run" | "installing" | "done">("idle");
  const [progressEvents, setProgressEvents] = useState<OperationProgressEvent[]>([]);

  useEffect(() => {
    setLoading(true); setDetails(null); setDownloadPlan(null); setError(null); setDownloadResult(null); setOperationPlan(null); setDryRun(null); setInstalled(null); setPhase("idle"); setProgressEvents([]);
    if (!applicationId) { setLoading(false); setError(`软件源中未找到 ${item.packageName} 的适配策略`); return; }
    void Promise.allSettled([getApplicationDetails(applicationId), getDownloadPlan(applicationId)]).then(([detailsOutcome, planOutcome]) => {
      if (detailsOutcome.status === "fulfilled") setDetails(detailsOutcome.value); else setError(String(detailsOutcome.reason));
      if (planOutcome.status === "fulfilled") setDownloadPlan(planOutcome.value); else setError(String(planOutcome.reason));
      setLoading(false);
    });
  }, [applicationId, item.packageName]);

  const isWebsite = details?.sourceKind === "officialWebsite";
  const hasUpdate = details?.updateState === "updateAvailable";
  const running = phase === "downloading" || phase === "planning" || phase === "dry-run" || phase === "installing";
  const targetVersion = details?.candidateVersion ?? downloadPlan?.version ?? "";

  const start = async () => {
    if (!applicationId) { setError(`软件源中未找到 ${item.packageName} 的适配策略`); return; }
    setError(null);
    setDownloadProgress(null); setDownloadResult(null); setOperationPlan(null); setDryRun(null); setInstalled(null); setProgressEvents([]);
    try {
      setPhase("downloading");
      const download = await downloadPackage(applicationId, setDownloadProgress);
      setDownloadResult(download);
      if (!download.verified) throw new Error("安装包校验未通过，已停止，未更改系统。");
      setPhase("ready");
      if (!document.hasFocus()) {
        void notifyDownloadComplete(`${item.displayName} 下载完成`, `安装包已通过校验，回到 UManager 继续更新 ${targetVersion}（需系统授权）。`).catch(() => { /* 通知失败不影响流程 */ });
      }
    } catch (reason) {
      setError(String(reason));
      setPhase("idle");
    }
  };

  const install = async () => {
    setError(null);
    try {
      setPhase("planning");
      const plan = await createOperationPlan(applicationId!);
      setOperationPlan(plan);
      setPhase("dry-run");
      const dry = await runOperationDryRun(plan.plan.planId);
      setDryRun(dry);
      setPhase("installing");
      const report = await installPackage(plan.plan.planId, (event) => appendProgress(setProgressEvents, event));
      setInstalled(report);
      onInstalled();
      setPhase("done");
    } catch (reason) {
      setError(String(reason));
      setPhase("ready");
    }
  };

  return <div className="drawer-layer" onMouseDown={(event) => { if (event.currentTarget === event.target && !running) onClose(); }}><aside className="detail-drawer" aria-label={`${item.displayName} 详情`}>
    <header className="drawer-header"><div className="drawer-app"><AppMark item={item}/><div><div className="drawer-title-row"><h2>{item.displayName}</h2><span>{item.vendor} · {item.packageName}</span></div>{catalogByPackage[item.packageName]?.description && <p className="drawer-description">{catalogByPackage[item.packageName]?.description}</p>}</div></div><button className="close-button" onClick={onClose} disabled={running} aria-label="关闭">×</button></header>
    {loading && <div className="drawer-loading"><span className="loader"/><p>正在读取官方源与安装包元数据…</p></div>}
    {error && !details && <div className="message error"><strong>无法检查 {item.displayName} 更新</strong><span>{error}</span></div>}
    {details && <div className="drawer-content">
      <div className={`trust-banner ${details.trusted ? "trusted" : "needsReview"}`}><Icon name="shield"/><div><strong>{details.trusted ? `来源验证通过 · ${sourceText[details.sourceKind] ?? details.sourceKind}` : "来源需要复核"}</strong><span>{isWebsite ? "官网、下载域名、Debian 包名和架构均符合软件源策略" : "包名、架构和官方仓库均符合软件源策略"}</span></div></div>
      <section className="detail-section"><h3>版本</h3><div className="version-pair"><div><span>已安装</span><strong>{details.installedVersion ?? "未安装"}</strong></div><div><span>{isWebsite ? "官方包完整版本" : "候选版本"}</span><strong>{details.candidateVersion ?? downloadPlan?.version ?? "待解析"}</strong></div></div><UpdatedAtLine seconds={details.versionUpdatedAtUnixSeconds} source={details.versionUpdatedAtSource}/>{details.websiteVersion && <p className="version-source-note">官网/发布标签展示 {details.websiteVersion}；UManager 通过 HTTP Range 读取 `.deb` 控制信息，获得用于比较的完整版本。</p>}</section>
      <section className="detail-section"><h3>官方来源</h3><div className="path-card"><div><span className={`source-dot ${details.sourceKind}`}/><strong>{isWebsite ? "厂商官方发布通道" : `${item.vendor} 官方 APT 仓库`}</strong></div><code title={details.sourceUrl}>{details.sourceUrl}</code><p>下载地址与所有 HTTPS 重定向都不能离开软件源中声明的允许域名。</p></div></section>
      <section className="detail-section"><h3>官方证据</h3><div className="evidence-list">{details.evidence.map((entry) => <div key={entry.label}><span className={entry.passed ? "check passed" : "check failed"}>{entry.passed ? "✓" : "!"}</span><div><strong>{entry.label}</strong><code>{entry.actual}</code></div></div>)}</div></section>
      <div className={`wechat-update-result ${details.updateState}`}><strong>{hasUpdate ? `发现新版本 ${details.candidateVersion}` : details.updateState === "upToDate" ? "已是最新版本" : "候选版本尚未确认"}</strong><span>{hasUpdate ? "点击下方按钮下载并校验官方包，校验通过后再确认安装。" : "官方完整版本与本机已安装版本相同。"}</span></div>
      <section className="detail-section download-section"><h3>官方安装包</h3>
        {downloadPlan && <DownloadCard plan={downloadPlan}/>}
        {phase === "downloading" && downloadProgress && <DownloadProgressCard progress={downloadProgress} displayName={item.displayName}/>}
        {downloadResult?.verified && <div className="download-success"><span>✓</span><div><strong>安装包校验通过</strong><p>大小、SHA-256、包名、版本和架构均通过；SHA-256：{downloadResult.actualSha256.slice(0, 16)}…</p></div></div>}
        {error && details && <div className="inline-error">{error}</div>}
        {hasUpdate && phase !== "done" && <button className="download-button" disabled={running || (phase === "idle" && !downloadPlan)} onClick={() => { if (phase === "ready") void install(); else void start(); }}>{phase === "downloading" ? "正在下载并校验…" : phase === "ready" ? "继续更新（需系统授权）" : phase === "planning" ? "正在生成不可变计划…" : phase === "dry-run" ? "正在特权环境复核…" : phase === "installing" ? "正在安装更新…" : `更新到 ${targetVersion}`}</button>}
        <p className="download-safety">下载不请求 root 权限；校验失败的文件不会进入缓存。下载完成后需再确认，才会请求系统授权并执行安装。</p>
      </section>
      {downloadResult?.verified && <section className="detail-section final-plan-section"><h3>最终更新计划</h3>
        <div className="final-plan-card"><dl><div><dt>动作</dt><dd>{isWebsite ? "install-verified-website-deb" : "install-verified-deb"}</dd></div><div><dt>包</dt><dd>{item.packageName} · {item.architecture}</dd></div><div><dt>版本</dt><dd>{item.installedVersion} → {downloadResult.version}</dd></div><div><dt>SHA-256</dt><dd title={downloadResult.actualSha256}>{downloadResult.actualSha256}</dd></div></dl></div>
        {operationPlan && <div className="immutable-plan"><strong>更新计划已锁定</strong><span>ID：{operationPlan.plan.planId}</span><span>有效至：{new Date(operationPlan.plan.payload.expiresAtUnixSeconds * 1000).toLocaleTimeString("zh-CN")}</span></div>}
        {dryRun && <div className="dry-run-success"><strong>✓ 特权环境复核通过</strong><span>helper 已重新核对来源、版本、架构和缓存中的安装包，本次未修改系统。</span></div>}
        <OperationLogPanel events={progressEvents} running={phase === "installing"}/>
        {installed && <div className="dry-run-success"><strong>✓ 更新安装完成</strong><span>固定参数 dpkg 安装命令已成功结束，软件列表已重新扫描。</span></div>}
      </section>}
    </div>}
  </aside></div>;
}

type MergedSoftware = {
  packageName: string;
  displayName: string;
  vendor: string;
  description: string | null;
  architecture: string;
  sourceKind: string;
  installed: boolean;
  installedVersion: string | null;
  candidateVersion: string | null;
  updateState: UpdateState;
  installAvailable: boolean;
  removable: boolean;
  managed?: ManagedPackage;
  offer?: InstallableApplication;
};

type SoftwareItem = {
  key: string;
  kind: "deb" | "devTool";
  category: string;
  displayName: string;
  vendor: string;
  description: string | null;
  deb?: MergedSoftware;
  tool?: DevTool;
  toolState?: DevToolState | null;
};

function SoftwareRow({ item, onOpen, onRemove, filter }: { item: MergedSoftware; onOpen: () => void; onRemove: () => void; filter: Filter }) {
  const catalogApp = catalogByPackage[item.packageName];
  const autoInstallable = !!catalogApp && isAutoInstallable(item.packageName);
  const canOpen = item.installed ? autoInstallable : item.installAvailable;
  const updateAvailable = item.installed && item.updateState === "updateAvailable" && autoInstallable;
  const showUpdate = filter === "updates" && updateAvailable;
  const statusText = item.installed
    ? item.updateState === "updateAvailable" ? "有可用更新" : item.updateState === "upToDate" ? "已是最新" : "需手动检查"
    : item.installAvailable ? "可安装" : "暂不可安装";
  const statusClass = item.installed ? item.updateState : item.installAvailable ? "updateAvailable" : "unknown";
  return <div className={`package-row ${canOpen ? "supported" : ""}`} role={canOpen ? "button" : undefined} tabIndex={canOpen ? 0 : undefined} onClick={() => { if (canOpen) onOpen(); }} onKeyDown={(event) => { if (canOpen && (event.key === "Enter" || event.key === " ")) onOpen(); }}>
    <div className="app-cell"><AppLogo packageName={item.packageName} displayName={item.displayName}/><div className="app-meta"><strong>{item.displayName}</strong>{item.description && <p className="app-description">{item.description}</p>}</div></div>
    <div className="version-cell"><strong>{item.installed ? item.installedVersion : "未安装"}</strong>{item.installed && item.updateState === "updateAvailable" && item.candidateVersion && <span>→ {item.candidateVersion}</span>}{!item.installed && item.installAvailable && item.candidateVersion && <span>可安装 {item.candidateVersion}</span>}</div>
    <div className="source-cell"><span className={`source-dot ${item.sourceKind}`}/><div><strong>{sourceText[item.sourceKind as keyof typeof sourceText] ?? item.sourceKind}</strong><span>{item.architecture}</span></div></div>
    <div className="status-cell"><span className={`status-badge ${statusClass}`}>{statusText}</span><div className="row-actions">
      {showUpdate && <button className="update-package-button" onClick={(event) => { event.stopPropagation(); onOpen(); }} onKeyDown={(event) => event.stopPropagation()} aria-label={`更新 ${item.displayName}`}>更新</button>}
      {!showUpdate && item.installed && item.removable && <button className="remove-package-button" onClick={(event) => { event.stopPropagation(); onRemove(); }} onKeyDown={(event) => event.stopPropagation()} aria-label={`卸载 ${item.displayName}`}>卸载</button>}
      {!item.installed && item.installAvailable && <button className="install-package-button" onClick={(event) => { event.stopPropagation(); onOpen(); }}>安装</button>}
      <span className={`row-arrow ${canOpen ? "" : "placeholder"}`} aria-hidden="true">›</span>
    </div></div>
  </div>;
}

function InstallDrawer({ offer, onClose, onInstalled }: { offer: InstallableApplication; onClose: () => void; onInstalled: () => void }) {
  const downloadPlan = offer.downloadPlan;
  const isWebsite = offer.sourceKind === "officialWebsite";
  const [downloadOutcome, setDownloadOutcome] = useState<{ actualSha256: string; version: string; reusedExistingFile: boolean; verified: boolean } | null>(null);
  const [downloadProgress, setDownloadProgress] = useState<DownloadProgress | null>(null);
  const [operationPlan, setOperationPlan] = useState<OperationPlanArtifact | null>(null);
  const [dryRun, setDryRun] = useState<OperationExecutionReport | null>(null);
  const [installed, setInstalled] = useState<OperationExecutionReport | null>(null);
  const [phase, setPhase] = useState<"idle" | "downloading" | "ready" | "planning" | "dry-run" | "installing" | "done">("idle");
  const [error, setError] = useState<string | null>(null);
  const [progressEvents, setProgressEvents] = useState<OperationProgressEvent[]>([]);

  const running = phase === "downloading" || phase === "planning" || phase === "dry-run" || phase === "installing";

  const start = async () => {
    setError(null);
    setDownloadProgress(null); setDownloadOutcome(null); setOperationPlan(null); setDryRun(null); setInstalled(null); setProgressEvents([]);
    try {
      setPhase("downloading");
      const result = await downloadPackage(offer.applicationId, setDownloadProgress);
      setDownloadOutcome({ actualSha256: result.actualSha256, version: result.version, reusedExistingFile: result.reusedExistingFile, verified: result.verified });
      if (!result.verified) throw new Error("安装包校验未通过，已停止，未更改系统。");
      setPhase("ready");
      if (!document.hasFocus()) {
        void notifyDownloadComplete(`${offer.displayName} 下载完成`, `安装包已通过校验，回到 UManager 继续安装（需系统授权）。`).catch(() => { /* 通知失败不影响流程 */ });
      }
    } catch (reason) {
      setError(String(reason));
      setPhase("idle");
    }
  };

  const install = async () => {
    setError(null);
    try {
      setPhase("planning");
      const plan = await createOperationPlan(offer.applicationId);
      setOperationPlan(plan);
      setPhase("dry-run");
      const dry = await runOperationDryRun(plan.plan.planId);
      setDryRun(dry);
      setPhase("installing");
      const report = await installPackage(plan.plan.planId, (event) => appendProgress(setProgressEvents, event));
      setInstalled(report);
      onInstalled();
      setPhase("done");
    } catch (reason) {
      setError(String(reason));
      setPhase("ready");
    }
  };

  return <div className="drawer-layer" onMouseDown={(event) => { if (event.currentTarget === event.target && !running) onClose(); }}><aside className="detail-drawer" aria-label={`安装 ${offer.displayName}`}>
    <header className="drawer-header"><div className="drawer-app"><AppLogo packageName={offer.packageName} displayName={offer.displayName}/><div><div className="drawer-title-row"><h2>{offer.displayName}</h2><span>{offer.vendor} · {offer.packageName}</span></div>{offer.description && <p className="drawer-description">{offer.description}</p>}</div></div><button className="close-button" onClick={onClose} disabled={running} aria-label="关闭">×</button></header>
    <div className="drawer-content">
      <div className="trust-banner trusted"><Icon name="shield"/><div><strong>来源验证通过 · {sourceText[offer.sourceKind] ?? offer.sourceKind}</strong><span>{offer.packageName} · {offer.architecture} 已匹配软件源策略</span></div></div>
      <section className="detail-section"><h3>新安装目标</h3><div className="version-pair"><div><span>当前状态</span><strong>未安装</strong></div><div><span>候选版本</span><strong>{offer.candidateVersion ?? "未解析"}</strong></div></div><UpdatedAtLine seconds={offer.versionUpdatedAtUnixSeconds} source={offer.versionUpdatedAtSource}/></section>
      <section className="detail-section"><h3>安装来源</h3><div className="path-card"><div><span className={`source-dot ${offer.sourceKind}`}/><strong>{isWebsite ? "厂商官方发布通道" : `${offer.vendor} 官方 APT 仓库`}</strong></div><code title={downloadPlan?.downloadUrl}>{downloadPlan?.downloadUrl}</code><p>{isWebsite ? "下载地址与所有 HTTPS 重定向都不能离开允许域名。" : "安装包路径和所有 HTTPS 重定向都不能离开允许域名。"}</p></div></section>
      <section className="detail-section download-section"><h3>官方安装包</h3>
        {downloadPlan && <DownloadCard plan={downloadPlan}/>}
        {phase === "downloading" && downloadProgress && <DownloadProgressCard progress={downloadProgress} displayName={offer.displayName}/>}
        {downloadOutcome?.verified && <div className="download-success"><span>✓</span><div><strong>安装包校验通过</strong><p>大小、SHA-256、包名、版本和架构均通过；SHA-256：{downloadOutcome.actualSha256.slice(0, 16)}…</p></div></div>}
        {error && <div className="inline-error">{error}</div>}
        {phase !== "done" && <button className="download-button" disabled={running || (phase === "idle" && !downloadPlan)} onClick={() => { if (phase === "ready") void install(); else void start(); }}>{phase === "downloading" ? "正在下载并校验…" : phase === "ready" ? "继续安装（需系统授权）" : phase === "planning" ? "正在生成不可变计划…" : phase === "dry-run" ? "正在特权环境复核…" : phase === "installing" ? "正在安装…" : `安装 ${offer.displayName}`}</button>}
        <p className="download-safety">下载不请求 root 权限；校验失败的文件不会进入缓存。下载完成后需再确认，才会请求系统授权并执行安装。</p>
      </section>
      {downloadOutcome?.verified && <section className="detail-section final-plan-section"><h3>最终安装计划</h3>
        <div className="final-plan-card"><dl><div><dt>动作</dt><dd>{isWebsite ? "install-verified-website-deb" : "install-verified-deb"}</dd></div><div><dt>包</dt><dd>{offer.packageName} · {offer.architecture}</dd></div><div><dt>版本</dt><dd>未安装 → {downloadOutcome.version}</dd></div><div><dt>SHA-256</dt><dd title={downloadOutcome.actualSha256}>{downloadOutcome.actualSha256}</dd></div></dl></div>
        {operationPlan && <div className="immutable-plan"><strong>安装计划已锁定</strong><span>ID：{operationPlan.plan.planId}</span><span>installedVersion：{operationPlan.plan.payload.installedVersion ?? "null（未安装）"}</span><span>有效至：{new Date(operationPlan.plan.payload.expiresAtUnixSeconds * 1000).toLocaleTimeString("zh-CN")}</span></div>}
        {dryRun && <div className="dry-run-success"><strong>✓ 特权环境复核通过</strong><span>helper 已确认软件仍未安装，并重新核对来源、缓存文件与安装包元数据。</span></div>}
        <OperationLogPanel events={progressEvents} running={phase === "installing"}/>
        {installed && <div className="dry-run-success"><strong>✓ {offer.displayName} 安装完成</strong><span>固定参数 dpkg 安装命令已成功结束，软件列表已重新扫描。</span></div>}
      </section>}
    </div>
  </aside></div>;
}

function DevLogPanel({ events, running }: { events: DevOperationProgress[]; running: boolean }) {
  if (events.length === 0) return null;
  const logs = events.filter((event) => event.phase === "running");
  return <section className="operation-progress-panel" aria-live="polite">
    <header><div><span className={running ? "operation-pulse" : "operation-complete-mark"}>{running ? "" : "✓"}</span><div><strong>{running ? "正在执行版本操作" : "版本操作已结束"}</strong><span>输出只读，不会请求 root 权限</span></div></div></header>
    <details className="operation-log-details" open={running || undefined}>
      <summary>详细日志 <b>{logs.length}</b></summary>
      <div className="operation-terminal" role="log">{logs.length === 0 ? <span className="terminal-placeholder">等待版本管理器输出…</span> : logs.map((event, index) => <div className={event.stream} key={`${index}-${event.message}`}><span>{event.stream === "stderr" ? "ERR" : "OUT"}</span><code>{event.message}</code></div>)}</div>
    </details>
  </section>;
}

function DevToolchainRow({ toolchain, onOpen }: { toolchain: DevToolchain; onOpen: () => void }) {
  const [state, setState] = useState<DevToolchainState | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setState(null); setError(null);
    getDevToolchainState(toolchain.toolchainId).then(setState).catch((reason) => setError(String(reason)));
  }, [toolchain.toolchainId]);
  const installedCount = state?.installedVersions.length ?? 0;
  const statusClass = state && !state.managerFound ? "unknown" : state && installedCount > 0 ? "upToDate" : "updateAvailable";
  const statusText = state && !state.managerFound ? `未检测到 ${state.manager}` : state && installedCount > 0 ? `已安装 ${installedCount} 个版本` : "可安装";
  return <div className="package-row supported" role="button" tabIndex={0} onClick={onOpen} onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") onOpen(); }}>
    <div className="app-cell"><DevLogo toolchain={toolchain}/><div className="app-meta"><strong>{toolchain.displayName}</strong><span>{toolchain.vendor} · {toolchain.manager}</span>{toolchain.description && <p className="app-description">{toolchain.description}</p>}</div></div>
    <div className="version-cell"><strong>{state?.defaultVersion ?? (state && !state.managerFound ? "未检测到" : "未设置")}</strong></div>
    <div className="source-cell"><span className="source-dot officialWebsite"/><div><strong>用户级版本管理器</strong><span>{toolchain.manager}</span></div></div>
    <div className="status-cell"><span className={`status-badge ${statusClass}`}>{statusText}</span>{error && <span className="row-arrow placeholder" aria-hidden="true">›</span>}</div>
  </div>;
}

function DevToolchainDrawer({ toolchain, onClose }: { toolchain: DevToolchain; onClose: () => void }) {
  const [state, setState] = useState<DevToolchainState | null>(null);
  const [releases, setReleases] = useState<DevRelease[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [events, setEvents] = useState<DevOperationProgress[]>([]);

  const refresh = async () => {
    setError(null);
    const [stateOutcome, releasesOutcome] = await Promise.allSettled([
      getDevToolchainState(toolchain.toolchainId),
      getDevReleases(toolchain.toolchainId),
    ]);
    if (stateOutcome.status === "fulfilled") setState(stateOutcome.value); else setError(String(stateOutcome.reason));
    if (releasesOutcome.status === "fulfilled") setReleases(releasesOutcome.value); else setError(String(releasesOutcome.reason));
  };
  useEffect(() => { void refresh(); }, [toolchain.toolchainId]);

  const run = async (version: string, action: (onProgress: (event: DevOperationProgress) => void) => Promise<DevOperationReport>) => {
    setBusy(version); setError(null); setEvents([]);
    try {
      await action((event) => setEvents((current) => [...current.slice(-199), event]));
      await refresh();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(null);
    }
  };

  const installedVersion = (version: string) => state?.installedVersions.some((item) => item.version === version || item.version.startsWith(`${version}-`));

  return <div className="drawer-layer" onMouseDown={(event) => { if (event.currentTarget === event.target && busy === null) onClose(); }}><aside className="detail-drawer" aria-label={`${toolchain.displayName} 开发环境详情`}>
    <header className="drawer-header"><div className="drawer-app"><DevLogo toolchain={toolchain}/><div><div className="drawer-title-row"><h2>{toolchain.displayName}</h2><span>{toolchain.vendor} · {toolchain.manager}</span></div>{toolchain.description && <p className="drawer-description">{toolchain.description}</p>}</div></div><button className="close-button" onClick={onClose} disabled={busy !== null} aria-label="关闭">×</button></header>
    <div className="drawer-content">
      {state && !state.managerFound && <div className="message"><strong>未检测到 {state.manager}</strong><span>版本管理器脚本缺失：{toolchain.managerHome}。请先安装 {state.manager}。</span></div>}
      {state?.managerFound && <div className="dev-facts">
        <div><dt>管理器</dt><dd>{state.manager} {state.managerVersion}</dd></div>
        <div><dt>默认版本</dt><dd>{state.defaultVersion ?? "未设置"}</dd></div>
        <div className="wide"><dt>管理器目录</dt><dd title={state.managerHome ?? undefined}>{state.managerHome}</dd></div>
      </div>}
      {error && <div className="inline-error">{error}</div>}

      {state?.managerFound && <div style={{ marginTop: 22 }}>
        <h3 className="dev-section-heading">已安装版本</h3>
        {state.installedVersions.length === 0 ? <p className="dev-empty">尚未安装任何版本。</p> : state.installedVersions.map((version) => <div key={version.version} className="dev-version-row">
          <div className="dev-version-name">
            <code>{version.version}</code>
            {version.isDefault && <span className="dev-badge default">默认</span>}
            {version.isLts && version.ltsName && <span className="dev-badge lts">LTS {version.ltsName}</span>}
          </div>
          <div className="dev-version-actions">
            {!version.isDefault && <button className="dev-action-button" disabled={busy !== null} onClick={() => void run(version.version, (onProgress) => setDevDefaultVersion(toolchain.toolchainId, version.version, onProgress))}>设为默认</button>}
            {!version.isDefault && <button className="dev-action-button danger" disabled={busy !== null} onClick={() => void run(version.version, (onProgress) => uninstallDevVersion(toolchain.toolchainId, version.version, onProgress))}>卸载</button>}
          </div>
        </div>)}
      </div>}

      {state?.managerFound && <div style={{ marginTop: 22 }}>
        <h3 className="dev-section-heading">可安装的 LTS 版本</h3>
        {!releases && <span className="loader"/>}
        {releases && releases.length === 0 && <p className="dev-empty">未能读取远程版本列表。</p>}
        {releases?.map((release) => <div key={release.version} className="dev-version-row">
          <div className="dev-version-name">
            <code>{release.version}</code>
            <span className="dev-sub-label">{release.label}</span>
            {release.recommended && <span className="dev-badge default">推荐</span>}
          </div>
          <div className="dev-version-actions">
            {installedVersion(release.version)
              ? <span className="dev-badge installed">已安装</span>
              : <button className="dev-action-button" disabled={busy !== null} onClick={() => void run(release.version, (onProgress) => installDevVersion(toolchain.toolchainId, release.version, onProgress))}>{busy === release.version ? "正在安装…" : "安装并设为默认"}</button>}
          </div>
        </div>)}
      </div>}

      <DevLogPanel events={events} running={busy !== null}/>
    </div>
  </aside></div>;
}

const devToolInstallKindText = { npmGlobal: "npm 全局安装", officialInstaller: "官方安装器", onPath: "本机 PATH 中的可执行文件" } as const;

function DevToolRow({ tool, state, onOpen }: { tool: DevTool; state: DevToolState | null; onOpen: () => void }) {
  const statusClass = state?.updateAvailable ? "updateAvailable" : state?.installed ? "upToDate" : "updateAvailable";
  const statusText = state?.updateAvailable ? "有可用更新" : state?.installed ? "已安装" : "可安装";
  return <div className="package-row supported" role="button" tabIndex={0} onClick={onOpen} onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") onOpen(); }}>
    <div className="app-cell"><DevToolLogo tool={tool}/><div className="app-meta"><strong>{tool.displayName}</strong>{tool.description && <p className="app-description">{tool.description}</p>}</div></div>
    <div className="version-cell"><strong>{state?.version ?? (state?.installed ? "已安装" : "未安装")}</strong>{state?.updateAvailable && <span>→ {state.latestVersion}</span>}</div>
    <div className="source-cell"><span className="source-dot officialWebsite"/><div><strong>{tool.installer.kind === "npm" ? "npm 全局" : "官方源"}</strong><span>{state?.installKind ? devToolInstallKindText[state.installKind] : "未安装"}</span></div></div>
    <div className="status-cell"><span className={`status-badge ${statusClass}`}>{statusText}</span><span className="row-arrow" aria-hidden="true">›</span></div>
  </div>;
}

function DevToolLogPanel({ events, running }: { events: DevToolProgress[]; running: boolean }) {
  if (events.length === 0) return null;
  const phases = events.filter((event) => event.phase !== "running");
  const logs = events.filter((event) => event.phase === "running");
  return <section className="operation-progress-panel" aria-live="polite">
    <header><div><span className={running ? "operation-pulse" : "operation-complete-mark"}>{running ? "" : "✓"}</span><div><strong>{running ? "正在执行 CLI 工具操作" : "CLI 工具操作已结束"}</strong><span>输出只读，不能输入或执行命令</span></div></div></header>
    <div className="operation-phase-list">{phases.map((event, index) => <div className={event.phase} key={`${index}-${event.message}`}><span>{event.phase === "completed" ? "✓" : "•"}</span><p>{event.message}</p></div>)}</div>
    <details className="operation-log-details" open={running || undefined}>
      <summary>详细日志 <b>{logs.length}</b></summary>
      <div className="operation-terminal" role="log">{logs.length === 0 ? <span className="terminal-placeholder">等待安装器输出…</span> : logs.map((event, index) => <div className={event.stream} key={`${index}-${event.message}`}><span>{event.stream === "stderr" ? "ERR" : "OUT"}</span><code>{event.message}</code></div>)}</div>
    </details>
  </section>;
}

function DevToolDrawer({ tool, onClose, onChanged }: { tool: DevTool; onClose: () => void; onChanged?: () => void }) {
  const [state, setState] = useState<DevToolState | null>(null);
  const [busy, setBusy] = useState<"install" | "uninstall" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [events, setEvents] = useState<DevToolProgress[]>([]);

  const refresh = async () => {
    setError(null);
    try { setState(await getDevToolState(tool.toolId)); } catch (reason) { setError(String(reason)); }
  };
  useEffect(() => { void refresh(); }, [tool.toolId]);

  const run = async (kind: typeof busy, action: (onProgress: (event: DevToolProgress) => void) => Promise<DevToolReport>) => {
    setBusy(kind); setError(null); setEvents([]);
    try {
      await action((event) => setEvents((current) => [...current.slice(-199), event]));
      await refresh();
      onChanged?.();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(null);
    }
  };

  const installLabel = tool.installer.kind === "npm" ? "通过 npm 全局安装" : "通过官方安装脚本安装";
  const canInstall = tool.installer.kind === "curlScript" || (state?.npmAvailable ?? false);

  return <div className="drawer-layer" onMouseDown={(event) => { if (event.currentTarget === event.target && busy === null) onClose(); }}><aside className="detail-drawer" aria-label={`${tool.displayName} 开发环境详情`}>
    <header className="drawer-header"><div className="drawer-app"><DevToolLogo tool={tool}/><div><div className="drawer-title-row"><h2>{tool.displayName}</h2><span>{tool.vendor} · {tool.binaryName}</span></div>{tool.description && <p className="drawer-description">{tool.description}</p>}</div></div><button className="close-button" onClick={onClose} disabled={busy !== null} aria-label="关闭">×</button></header>
    <div className="drawer-content">
      {state && !state.npmAvailable && <div className="message"><strong>未检测到 npm</strong><span>无法读取 npm 最新版本{tool.installer.kind === "npm" ? "，也无法安装该工具" : ""}。请先在“开发环境”安装并设置 Node.js。</span></div>}
      <div className="dev-facts">
        <div><dt>当前版本</dt><dd>{state?.version ?? "未安装"}</dd></div>
        <div><dt>最新版本</dt><dd>{state?.latestVersion ?? (state?.npmAvailable === false ? "无法读取" : "读取中…")}</dd></div>
        <div><dt>安装方式</dt><dd>{state?.installKind ? devToolInstallKindText[state.installKind] : "—"}</dd></div>
        <div className="wide"><dt>可执行文件</dt><dd title={state?.binaryPath ?? undefined}>{state?.binaryPath ?? (tool.installer.kind === "npm" ? `npm 包 ${tool.npmPackage}` : "官方安装脚本")}</dd></div>
      </div>
      {error && <div className="inline-error">{error}</div>}

      <div style={{ marginTop: 22 }}>
        <h3 className="dev-section-heading">操作</h3>
        {state?.installed
          ? <div className="dev-version-row">
              <div className="dev-version-name">
                <code>{state.version}</code>
                {state.updateAvailable && state.latestVersion && <span className="dev-sub-label">有更新 {state.latestVersion}</span>}
                {!state.updateAvailable && <span className="dev-badge installed">已是最新</span>}
              </div>
              <div className="dev-version-actions">
                {state.updateAvailable && <button className="dev-action-button" disabled={busy !== null || !canInstall} onClick={() => void run("install", (onProgress) => installDevTool(tool.toolId, onProgress))}>{busy === "install" ? "正在更新…" : `更新到 ${state.latestVersion}`}</button>}
                {state.canUninstall && <button className="dev-action-button danger" disabled={busy !== null} onClick={() => void run("uninstall", (onProgress) => uninstallDevTool(tool.toolId, onProgress))}>{busy === "uninstall" ? "正在卸载…" : "卸载"}</button>}
                {!state.canUninstall && <span className="dev-sub-label">未能确定安装来源，请按官方文档卸载</span>}
              </div>
            </div>
          : <div className="dev-version-row">
              <div className="dev-version-name"><code>未安装</code>{state?.latestVersion && <span className="dev-sub-label">最新 {state.latestVersion}</span>}</div>
              <div className="dev-version-actions">
                <button className="dev-action-button" disabled={busy !== null || !canInstall} onClick={() => void run("install", (onProgress) => installDevTool(tool.toolId, onProgress))}>{busy === "install" ? "正在安装…" : installLabel}</button>
              </div>
            </div>}
        {!canInstall && <p className="dev-empty">需要 npm 才能安装，请先在“开发环境”安装并设置 Node.js。</p>}
      </div>

      <DevToolLogPanel events={events} running={busy !== null}/>
    </div>
  </aside></div>;
}

function DevToolsPage() {
  const [toolchains, setToolchains] = useState<DevToolchain[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selectedToolchain, setSelectedToolchain] = useState<DevToolchain | null>(null);
  const refresh = async () => {
    setLoading(true); setError(null);
    try {
      setToolchains(await getDevToolchains());
    } catch (reason) {
      setError(String(reason));
    } finally {
      setLoading(false);
    }
  };
  useEffect(() => { void refresh(); }, []);
  return <main className="workspace dev-workspace">
    <header className="workspace-header"><div><h1>开发环境</h1><p>用户级版本管理器（Node.js / Rust）</p></div><div className="header-actions"><button className="primary-button" onClick={() => void refresh()} disabled={loading}><span className={loading ? "spin" : ""}>↻</span>{loading ? "检查中…" : "重新检查"}</button></div></header>
    <section className="software-panel">
      <div className="table-head"><span>软件</span><span>版本</span><span>来源</span><span>状态</span></div>
      {error && <div className="message error"><strong>无法读取开发环境</strong><span>{error}</span></div>}
      {loading && !toolchains && <div className="empty-state"><span className="loader"/><p>正在读取运行时状态…</p></div>}
      {toolchains && <div className="package-list">
        {toolchains.map((toolchain) => <DevToolchainRow toolchain={toolchain} key={toolchain.toolchainId} onOpen={() => setSelectedToolchain(toolchain)}/>)}
      </div>}
      {toolchains && toolchains.length === 0 && <div className="empty-state"><p>软件源中未配置运行时工具链。</p></div>}
    </section>
    {selectedToolchain && <DevToolchainDrawer toolchain={selectedToolchain} onClose={() => setSelectedToolchain(null)}/>}
  </main>;
}

function ScriptLogPanel({ events, running }: { events: ScriptProgressEvent[]; running: boolean }) {
  if (events.length === 0) return null;
  const system = events.filter((event) => event.stream === "system");
  const lines = events.filter((event) => event.stream !== "system");
  return <section className="operation-progress-panel" aria-live="polite">
    <header><div><span className={running ? "operation-pulse" : "operation-complete-mark"}>{running ? "" : "✓"}</span><div><strong>{running ? "脚本正在运行" : "脚本已结束"}</strong><span>输出只读，脚本以当前用户身份运行，不经 root</span></div></div></header>
    {system.length > 0 && <div className="script-system-lines">{system.map((event, index) => <div key={index}>{event.message}</div>)}</div>}
    <div className="operation-terminal" role="log">{lines.length === 0 ? <span className="terminal-placeholder">等待脚本输出…</span> : lines.map((event, index) => <div className={event.stream} key={index}><span>{event.stream === "stderr" ? "ERR" : "OUT"}</span><code>{event.message}</code></div>)}</div>
  </section>;
}

function ScriptsPage() {
  const [scripts, setScripts] = useState<ScriptDefinition[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [runningId, setRunningId] = useState<string | null>(null);
  const [logScriptId, setLogScriptId] = useState<string | null>(null);
  const [events, setEvents] = useState<ScriptProgressEvent[]>([]);

  const refresh = async () => {
    setLoading(true); setError(null);
    try { setScripts(await listScripts()); } catch (reason) { setError(String(reason)); } finally { setLoading(false); }
  };
  useEffect(() => { void refresh(); }, []);

  const run = async (script: ScriptDefinition, action: ScriptAction) => {
    setRunningId(script.id); setError(null); setLogScriptId(script.id); setEvents([]);
    try {
      await runScript(script.id, action.id, (event) => setEvents((current) => [...current.slice(-499), event]));
    } catch (reason) {
      setError(String(reason));
    } finally {
      setRunningId((current) => (current === script.id ? null : current));
    }
  };

  const stop = async (scriptId: string) => {
    try { await stopScript(scriptId); } catch (reason) { setError(String(reason)); }
  };

  return <main className="workspace scripts-workspace">
    <header className="workspace-header"><div><h1>维护脚本</h1><p>内置维护脚本 · 以当前用户身份运行，不需要 root</p></div><div className="header-actions"><button className="primary-button" onClick={() => void refresh()} disabled={loading}><span className={loading ? "spin" : ""}>↻</span>{loading ? "读取中…" : "重新读取"}</button></div></header>
    <section className="software-panel script-panel">
      {error && <div className="message error"><strong>无法读取脚本</strong><span>{error}</span></div>}
      {loading && !scripts && <div className="empty-state"><span className="loader"/><p>正在读取内置脚本…</p></div>}
      {scripts && scripts.length === 0 && <div className="empty-state"><p>没有可用的内置脚本。</p></div>}
      {scripts && scripts.map((script) => <div className="script-card" key={script.id}>
        <div className="script-card-head">
          <div className="app-meta"><strong>{script.name}</strong><span className="script-meta">{script.id}</span>{script.description && <p className="app-description">{script.description}</p>}</div>
          <span className="status-badge upToDate">用户级 · 无 root</span>
        </div>
        <div className="script-actions">
          {runningId === script.id
            ? <span className="script-running-label">运行中…</span>
            : script.actions.map((action) => <button key={action.id} className="dev-action-button" disabled={runningId !== null} onClick={() => void run(script, action)}>{action.label}</button>)}
          {runningId === script.id && <button className="dev-action-button danger" onClick={() => void stop(script.id)}>停止</button>}
        </div>
        {logScriptId === script.id && events.length > 0 && <ScriptLogPanel events={events} running={runningId === script.id}/>}
      </div>)}
    </section>
  </main>;
}

export default function App() {
  const [page, setPage] = useState<Page>("installed");
  const [result, setResult] = useState<ScanResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const [filter, setFilter] = useState<Filter>("all");
  const [categoryFilter, setCategoryFilter] = useState<string>("全部");
  const [query, setQuery] = useState("");
  const [updatePackage, setUpdatePackage] = useState<ManagedPackage | null>(null);
  const [removalPackage, setRemovalPackage] = useState<ManagedPackage | null>(null);
  const [installableOffers, setInstallableOffers] = useState<InstallableApplication[] | null>(null);
  const [installableLoading, setInstallableLoading] = useState(false);
  const [installableError, setInstallableError] = useState<string | null>(null);
  const [installOffer, setInstallOffer] = useState<InstallableApplication | null>(null);
  const [installationInfo, setInstallationInfo] = useState<InstallationInfo | null>(null);
  const [installationInfoLoading, setInstallationInfoLoading] = useState(false);
  const [installationInfoError, setInstallationInfoError] = useState<string | null>(null);
  const [appVersion, setAppVersion] = useState<string | null>(null);
  const [selfRemovalOpen, setSelfRemovalOpen] = useState(false);
  const [selfUpdateOpen, setSelfUpdateOpen] = useState(false);
  const [pendingLocalDeb, setPendingLocalDeb] = useState<LocalDebInspection | null>(null);
  const [pendingLocalDebError, setPendingLocalDebError] = useState<string | null>(null);
  const [catalog, setCatalog] = useState<CatalogApplication[] | null>(null);
  const [categoryCatalog, setCategoryCatalog] = useState<CategoryCatalog | null>(null);
  const [devTools, setDevTools] = useState<DevTool[] | null>(null);
  const [devToolStates, setDevToolStates] = useState<Record<string, DevToolState | null>>({});
  const [devToolsError, setDevToolsError] = useState<string | null>(null);
  const [selectedDevTool, setSelectedDevTool] = useState<DevTool | null>(null);

  useEffect(() => {
    void getSoftwareCatalog().then((entries) => {
      catalogByPackage = Object.fromEntries(entries.map((entry) => [entry.packageName, entry]));
      setCatalog(entries);
    }).catch(() => setCatalog([]));
  }, []);

  const refresh = async () => {
    setScanning(true); setError(null);
    try { setResult(await scanPackages()); } catch (reason) { setError(String(reason)); } finally { setScanning(false); }
  };
  const refreshInstallable = async () => {
    setInstallableLoading(true); setInstallableError(null);
    try { setInstallableOffers(await getInstallableApplications()); } catch (reason) { setInstallableError(String(reason)); } finally { setInstallableLoading(false); }
  };
  const refreshInstallationInfo = async () => {
    setInstallationInfoLoading(true); setInstallationInfoError(null);
    try { setInstallationInfo(await getInstallationInfo()); } catch (reason) { setInstallationInfoError(String(reason)); } finally { setInstallationInfoLoading(false); }
  };
  const loadCategories = () => {
    getCategories().then(setCategoryCatalog).catch(() => setCategoryCatalog(null));
  };
  const loadDevTools = async () => {
    setDevToolsError(null);
    try {
      const tools = await getDevTools();
      setDevTools(tools);
      const states = await Promise.all(tools.map(async (tool) => {
        try { return [tool.toolId, await getDevToolState(tool.toolId)] as const; }
        catch { return [tool.toolId, null] as const; }
      }));
      setDevToolStates(Object.fromEntries(states));
    } catch (reason) {
      setDevToolsError(String(reason));
    }
  };
  useEffect(() => {
    void refresh();
    void refreshInstallable();
    void refreshInstallationInfo();
    void loadDevTools();
    void loadCategories();
    void getPendingLocalDeb().then(setPendingLocalDeb).catch((reason) => setPendingLocalDebError(String(reason)));
    getVersion().then(setAppVersion).catch(() => { /* 获取编译版本失败时兜底为空 */ });
  }, []);

  const softwareItems = useMemo(() => {
    const items: SoftwareItem[] = [];
    for (const mg of result?.packages ?? []) {
      const applicationId = catalogByPackage[mg.packageName]?.applicationId;
      items.push({
        key: `deb-${mg.packageName}`,
        kind: "deb",
        category: debCategory(categoryCatalog, applicationId),
        displayName: mg.displayName,
        vendor: mg.vendor,
        description: catalogByPackage[mg.packageName]?.description ?? null,
        deb: {
          packageName: mg.packageName, displayName: mg.displayName, vendor: mg.vendor,
          description: catalogByPackage[mg.packageName]?.description ?? null,
          architecture: mg.architecture, sourceKind: mg.sourceKind,
          installed: true, installedVersion: mg.installedVersion, candidateVersion: mg.candidateVersion,
          updateState: mg.updateState, installAvailable: false, removable: catalogByPackage[mg.packageName]?.removable ?? false,
          managed: mg, offer: undefined,
        },
      });
    }
    for (const off of installableOffers ?? []) {
      const existing = items.find((item) => item.kind === "deb" && item.deb?.packageName === off.packageName);
      if (existing?.deb) {
        const deb = existing.deb;
        deb.offer = off;
        deb.description = deb.description ?? off.description ?? null;
        deb.candidateVersion = deb.candidateVersion ?? off.candidateVersion;
        deb.architecture = deb.architecture ?? off.architecture;
        deb.sourceKind = deb.sourceKind ?? off.sourceKind;
        deb.installAvailable = off.installAvailable;
      } else {
        items.push({
          key: `deb-${off.packageName}`,
          kind: "deb",
          category: debCategory(categoryCatalog, off.applicationId),
          displayName: off.displayName,
          vendor: off.vendor,
          description: off.description ?? catalogByPackage[off.packageName]?.description ?? null,
          deb: {
            packageName: off.packageName, displayName: off.displayName, vendor: off.vendor,
            description: off.description ?? catalogByPackage[off.packageName]?.description ?? null,
            architecture: off.architecture, sourceKind: off.sourceKind,
            installed: off.installedVersion != null, installedVersion: off.installedVersion, candidateVersion: off.candidateVersion,
            updateState: "unknown", installAvailable: off.installAvailable, removable: catalogByPackage[off.packageName]?.removable ?? false,
            managed: undefined, offer: off,
          },
        });
      }
    }
    for (const tool of devTools ?? []) {
      items.push({
        key: `devtool-${tool.toolId}`,
        kind: "devTool",
        category: devToolCategory(categoryCatalog, tool.toolId),
        displayName: tool.displayName,
        vendor: tool.vendor,
        description: tool.description ?? null,
        tool,
        toolState: devToolStates[tool.toolId] ?? null,
      });
    }
    return items.sort((a, b) => a.displayName.localeCompare(b.displayName, "zh-CN"));
  }, [result, installableOffers, devTools, devToolStates, categoryCatalog]);
  const presentCategories = useMemo(() => {
    const set = new Set<string>();
    for (const item of softwareItems) set.add(item.category);
    return set;
  }, [softwareItems]);
  const categoryChips = useMemo(() => orderedCategories(categoryCatalog, presentCategories), [categoryCatalog, presentCategories]);
  const updatesCount = useMemo(() => softwareItems.filter((item) => item.kind === "deb" ? item.deb!.updateState === "updateAvailable" : item.toolState?.updateAvailable === true).length, [softwareItems]);
  const visibleSoftware = useMemo(() => softwareItems.filter((item) => {
    const searchable = `${item.displayName} ${item.vendor}${item.kind === "deb" ? ` ${item.deb!.packageName}` : ""}`.toLowerCase();
    const textMatch = searchable.includes(query.toLowerCase());
    const categoryMatch = categoryFilter === "全部" || item.category === categoryFilter;
    const stateMatch = (() => {
      if (filter === "all") return true;
      if (item.kind === "deb") {
        const deb = item.deb!;
        if (filter === "installed") return deb.installed;
        if (filter === "updates") return deb.updateState === "updateAvailable";
        return !deb.installed;
      }
      const state = item.toolState;
      if (filter === "installed") return state?.installed === true;
      if (filter === "updates") return state?.updateAvailable === true;
      return !state || state.installed !== true;
    })();
    return textMatch && categoryMatch && stateMatch;
  }), [softwareItems, filter, query, categoryFilter]);
  const openSoftware = (item: SoftwareItem) => {
    if (item.kind === "devTool") {
      if (item.tool) setSelectedDevTool(item.tool);
      return;
    }
    const deb = item.deb!;
    if (deb.installed) {
      if (deb.managed && isAutoInstallable(deb.packageName)) setUpdatePackage(deb.managed);
    } else if (deb.offer) {
      setInstallOffer(deb.offer);
    }
  };
  const refreshAll = () => { void refresh(); void refreshInstallable(); void loadDevTools(); void loadCategories(); };
  const showInstalledPage = () => {
    setPage("installed");
    setInstallOffer(null); setUpdatePackage(null); setSelectedDevTool(null);
    if (!installableOffers && !installableLoading) void refreshInstallable();
  };
  const showDevToolsPage = () => {
    setPage("dev");
    setUpdatePackage(null); setRemovalPackage(null); setInstallOffer(null); setSelectedDevTool(null);
  };
  const showScriptsPage = () => {
    setPage("scripts");
    setUpdatePackage(null); setRemovalPackage(null); setInstallOffer(null); setSelectedDevTool(null);
  };
  const showSettingsPage = () => {
    setPage("settings");
    setUpdatePackage(null); setRemovalPackage(null); setInstallOffer(null); setSelectedDevTool(null);
    if (!installationInfo && !installationInfoLoading) void refreshInstallationInfo();
  };

  return <div className="app-shell">
    <aside className="sidebar">
      <div className="brand"><span className="brand-mark">U</span><strong>UManager</strong></div>
      <nav aria-label="主导航">
        <button className={`nav-item ${page === "installed" ? "active" : ""}`} onClick={showInstalledPage}><Icon name="apps"/>软件管家</button>
        <button className={`nav-item ${page === "scripts" ? "active" : ""}`} onClick={showScriptsPage}><Icon name="script"/>维护脚本</button>
        <button className={`nav-item ${page === "dev" ? "active" : ""}`} onClick={showDevToolsPage}><Icon name="dev"/>开发环境</button>
        <button className="nav-item" disabled><Icon name="history"/>操作记录<span className="later">稍后</span></button>
      </nav>
      <div className="sidebar-spacer"/>
      <div className="safety-card"><Icon name="shield"/><div><strong>安全更新</strong><span>仅在确认授权后更改系统</span></div></div>
      <button className={`nav-item settings ${page === "settings" ? "active" : ""}`} onClick={showSettingsPage}><Icon name="settings"/>设置</button>
      <div className="version-label">UManager {installationInfo?.appVersion ?? appVersion ?? ""}</div>
    </aside>

    {page === "installed" ? <main className="workspace">
      <header className="workspace-header"><div><h1>软件管家</h1><p>桌面应用、命令行工具与 AI 工具</p></div><div className="header-actions">{result && <time>上次检查 {new Date(result.scannedAtUnixSeconds * 1000).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })}</time>}<button className="primary-button" onClick={refreshAll} disabled={scanning}><span className={scanning ? "spin" : ""}>↻</span>{scanning ? "检查中…" : "检查更新"}</button></div></header>
      <section className="software-panel">
        {pendingLocalDebError && <div className="message error"><strong>无法打开本地 .deb</strong><span>{pendingLocalDebError}</span></div>}
        <div className="category-bar">
          <button className={`category-chip ${categoryFilter === "全部" ? "active" : ""}`} onClick={() => setCategoryFilter("全部")}>全部</button>
          {categoryChips.map((category) => <button key={category} className={`category-chip ${categoryFilter === category ? "active" : ""}`} onClick={() => setCategoryFilter(category)}>{category}</button>)}
        </div>
        <div className="panel-toolbar"><div className="filter-tabs">
          <button className={filter === "all" ? "active" : ""} onClick={() => setFilter("all")}>全部</button>
          <button className={filter === "installed" ? "active" : ""} onClick={() => setFilter("installed")}>已安装</button>
          <button className={filter === "updates" ? "active" : ""} onClick={() => setFilter("updates")}>可更新 {updatesCount > 0 && <b>{updatesCount}</b>}</button>
          <button className={filter === "installable" ? "active" : ""} onClick={() => setFilter("installable")}>可安装</button>
        </div><label className="search-box"><Icon name="search"/><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索软件"/></label></div>
        <div className="table-head"><span>软件</span><span>版本</span><span>来源</span><span>状态</span></div>
        {error && <div className="message error"><strong>无法读取软件信息</strong><span>{error}</span></div>}
        {installableError && <div className="message error"><strong>无法读取可安装软件</strong><span>{installableError}</span></div>}
        {devToolsError && <div className="message error"><strong>无法读取 CLI 工具</strong><span>{devToolsError}</span></div>}
        {result?.warnings.map((warning) => <div className="message" key={warning}>{warning}</div>)}
        {!result && !installableOffers && !devTools && <div className="empty-state"><span className="loader"/><p>正在读取已安装与可安装软件…</p></div>}
        {result && visibleSoftware.length === 0 && <div className="empty-state"><p>没有符合条件的软件</p></div>}
        <div className="package-list">{visibleSoftware.map((item) => item.kind === "deb"
          ? <SoftwareRow item={item.deb!} onOpen={() => openSoftware(item)} onRemove={() => { if (item.deb!.managed) setRemovalPackage(item.deb!.managed); }} filter={filter} key={item.key}/>
          : <DevToolRow tool={item.tool!} state={item.toolState ?? null} onOpen={() => openSoftware(item)} key={item.key}/>)}</div>
      </section>
    </main> : page === "dev" ? <DevToolsPage/> : page === "scripts" ? <ScriptsPage/> : <SettingsPage info={installationInfo} loading={installationInfoLoading} error={installationInfoError} onRefresh={() => void refreshInstallationInfo()} onRemove={() => setSelfRemovalOpen(true)} onUpdate={() => setSelfUpdateOpen(true)}/>}
    {updatePackage && <UpdateDrawer item={updatePackage} onClose={() => setUpdatePackage(null)} onInstalled={() => void refresh()}/>}
    {pendingLocalDeb && <LocalDebDialog initial={pendingLocalDeb} onClose={() => setPendingLocalDeb(null)} onInstalled={() => void refresh()}/>}
    {removalPackage && <RemovalDialog item={removalPackage} onClose={() => setRemovalPackage(null)} onRemoved={() => void refresh()}/>}
    {installOffer && (
      <InstallDrawer offer={installOffer} onClose={() => setInstallOffer(null)} onInstalled={() => { void refresh(); void refreshInstallable(); }}/>
    )}
    {selectedDevTool && (
      <DevToolDrawer tool={selectedDevTool} onClose={() => setSelectedDevTool(null)} onChanged={() => void loadDevTools()}/>
    )}
    {selfRemovalOpen && installationInfo && (
      <SelfRemovalDialog info={installationInfo} onClose={() => setSelfRemovalOpen(false)}/>
    )}
    {selfUpdateOpen && installationInfo && (
      <SelfUpdateDialog info={installationInfo} onClose={() => setSelfUpdateOpen(false)} onUpdated={() => void refreshInstallationInfo()}/>
    )}
  </div>;
}
