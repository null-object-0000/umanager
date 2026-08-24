import { useEffect, useMemo, useState } from "react";
import { createLocalDebOperationPlan, createVscodeOperationPlan, downloadVscodePackage, getPendingLocalDeb, getVscodeDetails, getVscodeDownloadPlan, getWechatDetails, importPendingLocalDeb, installLocalDeb, runLocalDebDryRun, runVscodeOperationDryRun, scanPackages } from "./api";
import { summarizePackages } from "./model";
import type { DryRunReport, LocalDebInspection, ManagedPackage, OperationExecutionReport, OperationPlanArtifact, ScanResult, VscodeDetails, VscodeDownloadPlan, VscodeDownloadResult, WechatDetails } from "./types";
import chatgptIcon from "./assets/app-icons/chatgpt.png";
import flclashIcon from "./assets/app-icons/flclash.png";
import chromeIcon from "./assets/app-icons/google-chrome.png";
import vscodeIcon from "./assets/app-icons/vscode.png";
import wechatIcon from "./assets/app-icons/wechat.png";
import wemeetIcon from "./assets/app-icons/wemeet.svg";

type Filter = "all" | "updates" | "local";
const sourceText = { officialRepository: "官方 APT 仓库", localPackage: "本地 .deb" } as const;
const appColors: Record<string, string> = { code: "#2b78bd", "google-chrome-stable": "#4285f4", chatgpt: "#171918", flclash: "#7c5ce5", wechat: "#22ad38", wemeet: "#2878ff" };
const appIcons: Record<string, string> = { code: vscodeIcon, "google-chrome-stable": chromeIcon, chatgpt: chatgptIcon, flclash: flclashIcon, wechat: wechatIcon, wemeet: wemeetIcon };

function Icon({ name }: { name: "apps" | "source" | "history" | "settings" | "search" | "shield" }) {
  const paths = {
    apps: <><rect x="3" y="3" width="7" height="7" rx="2"/><rect x="14" y="3" width="7" height="7" rx="2"/><rect x="3" y="14" width="7" height="7" rx="2"/><rect x="14" y="14" width="7" height="7" rx="2"/></>,
    source: <><path d="M4 7h16M6 3h12l2 4-2 4H6L4 7l2-4Z"/><path d="M7 11v10m10-10v10M4 21h16"/></>,
    history: <><path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5M12 7v5l3 2"/></>,
    settings: <><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.9l.1.1-2.8 2.8-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.6v.2h-4V21a1.7 1.7 0 0 0-1-1.6 1.7 1.7 0 0 0-1.9.3l-.1.1L4.2 17l.1-.1a1.7 1.7 0 0 0 .3-1.9A1.7 1.7 0 0 0 3 14H2.8v-4H3a1.7 1.7 0 0 0 1.6-1 1.7 1.7 0 0 0-.3-1.9L4.2 7 7 4.2l.1.1a1.7 1.7 0 0 0 1.9.3A1.7 1.7 0 0 0 10 3V2.8h4V3a1.7 1.7 0 0 0 1 1.6 1.7 1.7 0 0 0 1.9-.3l.1-.1L19.8 7l-.1.1a1.7 1.7 0 0 0-.3 1.9 1.7 1.7 0 0 0 1.6 1h.2v4H21a1.7 1.7 0 0 0-1.6 1Z"/></>,
    search: <><circle cx="11" cy="11" r="7"/><path d="m20 20-4-4"/></>,
    shield: <><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10Z"/><path d="m9 12 2 2 4-4"/></>,
  };
  return <svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true">{paths[name]}</svg>;
}

function AppLogo({ packageName, displayName }: { packageName: string; displayName: string }) {
  const icon = appIcons[packageName];
  const letter = displayName === "微信" ? "微" : displayName === "腾讯会议" ? "会" : displayName.slice(0, 1).toUpperCase();
  return <span className={`app-mark ${icon ? "has-icon" : ""}`} style={{ background: icon ? undefined : appColors[packageName] ?? "#555" }}>{icon ? <img src={icon} alt=""/> : letter}</span>;
}

function AppMark({ item }: { item: ManagedPackage }) {
  return <AppLogo packageName={item.packageName} displayName={item.displayName}/>;
}

function PackageRow({ item, onOpen }: { item: ManagedPackage; onOpen?: () => void }) {
  const hasUpdate = item.updateState === "updateAvailable";
  return <div className={`package-row ${onOpen ? "supported" : ""}`} role={onOpen ? "button" : undefined} tabIndex={onOpen ? 0 : undefined} onClick={onOpen} onKeyDown={(event) => { if (onOpen && (event.key === "Enter" || event.key === " ")) onOpen(); }}>
    <div className="app-cell"><AppMark item={item}/><div className="app-meta"><strong>{item.displayName}</strong><span>{item.vendor} · {item.packageName}</span></div></div>
    <div className="version-cell"><strong>{item.installedVersion}</strong>{hasUpdate && <span>→ {item.candidateVersion}</span>}</div>
    <div className="source-cell"><span className={`source-dot ${item.sourceKind}`}/><div><strong>{sourceText[item.sourceKind]}</strong><span>{item.architecture}</span></div></div>
    <div className="status-cell"><span className={`status-badge ${item.updateState}`}>{hasUpdate ? "有可用更新" : item.updateState === "upToDate" ? "已是最新" : "需手动检查"}</span>{onOpen && <span className="row-arrow">›</span>}</div>
  </div>;
}

function formatBytes(bytes: number) {
  return `${(bytes / 1024 / 1024).toFixed(1)} MiB`;
}

function LocalDebDialog({ initial, onClose, onInstalled }: { initial: LocalDebInspection; onClose: () => void; onInstalled: () => void }) {
  const [inspected, setInspected] = useState(initial);
  const [confirmed, setConfirmed] = useState(false);
  const [plan, setPlan] = useState<OperationPlanArtifact | null>(null);
  const [dryRun, setDryRun] = useState<OperationExecutionReport | null>(null);
  const [installed, setInstalled] = useState<OperationExecutionReport | null>(null);
  const [busy, setBusy] = useState<"import" | "plan" | "dry-run" | "install" | null>(null);
  const [error, setError] = useState<string | null>(null);
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
        {plan && dryRun && !installed && <><div className="dry-run-success"><strong>✓ 安装前复核通过</strong><span>helper 已独立复核路径、包名、版本、架构、大小和 SHA-256。</span></div><button className="install-local-button" disabled={busy !== null} onClick={() => void run("install", () => installLocalDeb(plan!.plan.planId), (value) => { setInstalled(value); onInstalled(); })}>{busy === "install" ? "正在安装…" : `授权并安装 ${inspected.packageName}`}</button></>}
        {installed && <div className="dry-run-success"><strong>✓ 安装命令已成功完成</strong><span>已重新扫描；若该软件已有 UManager 厂商适配，它会出现在受管列表中。</span></div>}
      </div>
    </section>
  </div>;
}

function WechatDrawer({ details, loading, error, onClose }: { details: WechatDetails | null; loading: boolean; error: string | null; onClose: () => void }) {
  return <div className="drawer-layer" onMouseDown={(event) => { if (event.currentTarget === event.target) onClose(); }}><aside className="detail-drawer" aria-label="微信详情">
    <header className="drawer-header"><div className="drawer-app"><AppLogo packageName="wechat" displayName="微信"/><div><h2>微信</h2><span>官网稳定下载通道</span></div></div><button className="close-button" onClick={onClose} aria-label="关闭">×</button></header>
    {loading && <div className="drawer-loading"><span className="loader"/><p>正在读取官网与安装包元数据…</p></div>}
    {error && <div className="message error"><strong>无法检查微信更新</strong><span>{error}</span></div>}
    {details && <div className="drawer-content">
      <div className="trust-banner trusted"><Icon name="shield"/><div><strong>官方通道验证通过</strong><span>官网、下载域名、Debian 包名和架构均符合适配策略</span></div></div>
      <section className="detail-section"><h3>版本</h3><div className="version-pair"><div><span>已安装</span><strong>{details.installedVersion}</strong></div><div><span>官方包完整版本</span><strong>{details.packageVersion}</strong></div></div><p className="version-source-note">官网页面展示 {details.websiteVersion}；UManager 通过 HTTP Range 读取 `.deb` 控制信息，获得用于比较的完整版本。</p></section>
      <section className="detail-section"><h3>官方下载通道</h3><div className="path-card"><div><span className="source-dot officialRepository"/><strong>微信 Linux 官网固定 x86_64 地址</strong></div><code title={details.downloadUrl}>{details.downloadUrl}</code><p>完整文件 {formatBytes(details.expectedSize)}；检查更新时只读取前 8 MiB 控制信息，不会下载整包。</p></div></section>
      <section className="detail-section"><h3>官方证据</h3><div className="evidence-list">{details.evidence.map((item) => <div key={item.label}><span className="check passed">✓</span><div><strong>{item.label}</strong><code>{item.actual}</code></div></div>)}</div></section>
      <div className={`wechat-update-result ${details.updateState}`}><strong>{details.updateState === "updateAvailable" ? `发现新版本 ${details.packageVersion}` : "已是最新版本"}</strong><span>{details.updateState === "updateAvailable" ? "下一步将接入完整下载、SHA-256 锁定与特权安装计划。" : "官方包完整版本与本机已安装版本相同。"}</span></div>
    </div>}
  </aside></div>;
}

interface VscodeDrawerProps {
  details: VscodeDetails | null;
  loading: boolean;
  error: string | null;
  downloadPlan: VscodeDownloadPlan | null;
  planError: string | null;
  downloading: boolean;
  downloadResult: VscodeDownloadResult | null;
  downloadError: string | null;
  operationConfirmed: boolean;
  operationPlan: OperationPlanArtifact | null;
  operationLoading: boolean;
  operationError: string | null;
  dryRunRunning: boolean;
  dryRunResult: DryRunReport | null;
  dryRunError: string | null;
  onConfirmationChange: (confirmed: boolean) => void;
  onCreateOperationPlan: () => void;
  onDryRun: () => void;
  onDownload: () => void;
  onClose: () => void;
}

function VscodeDrawer({ details, loading, error, downloadPlan, planError, downloading, downloadResult, downloadError, operationConfirmed, operationPlan, operationLoading, operationError, dryRunRunning, dryRunResult, dryRunError, onConfirmationChange, onCreateOperationPlan, onDryRun, onDownload, onClose }: VscodeDrawerProps) {
  return <div className="drawer-layer" onMouseDown={(event) => { if (event.currentTarget === event.target) onClose(); }}>
    <aside className="detail-drawer" aria-label="Visual Studio Code 详情">
      <header className="drawer-header"><div className="drawer-app"><AppLogo packageName="code" displayName="Visual Studio Code"/><div><h2>Visual Studio Code</h2><span>完整只读适配</span></div></div><button className="close-button" onClick={onClose} aria-label="关闭">×</button></header>
      {loading && <div className="drawer-loading"><span className="loader"/><p>正在重新核对本机证据…</p></div>}
      {error && <div className="message error"><strong>无法读取 VS Code 详情</strong><span>{error}</span></div>}
      {details && <div className="drawer-content">
        <div className={`trust-banner ${details.trustState}`}><Icon name="shield"/><div><strong>{details.trustState === "trusted" ? "来源验证通过" : "来源需要复核"}</strong><span>{details.trustState === "trusted" ? "包名、架构和官方仓库均符合适配策略" : "部分来源证据与适配策略不一致"}</span></div></div>
        <section className="detail-section"><h3>版本</h3><div className="version-pair"><div><span>已安装</span><strong>{details.installedVersion}</strong></div><div><span>候选版本</span><strong>{details.candidateVersion ?? "待解析"}</strong></div></div></section>
        <section className="detail-section"><h3>更新路径</h3><div className="path-card"><div><span className="source-dot officialRepository"/><strong>{details.selectedPath.label}</strong></div><code>{details.selectedPath.endpoint}</code><p>{details.selectedPath.reason}</p></div><details><summary>备用官方接口</summary><code className="fallback-url">{details.fallbackEndpoint}</code></details></section>
        <section className="detail-section"><h3>本机信任证据</h3><div className="evidence-list">{details.evidence.map((item) => <div key={item.label}><span className={item.passed ? "check passed" : "check failed"}>{item.passed ? "✓" : "!"}</span><div><strong>{item.label}</strong><code>{item.actual}</code></div></div>)}</div></section>
        <section className="detail-section"><h3>下载后校验</h3><div className="verification-grid">{details.verificationPlan.map((item) => <div key={item.label}><span>{item.label}</span><strong>{item.expected}</strong><em className={item.state}>{item.state === "passed" ? "已确认" : "计划校验"}</em></div>)}</div></section>
        <section className="detail-section download-section"><h3>官方安装包</h3>
          {planError && <div className="inline-error">{planError}</div>}
          {downloadPlan && <div className="download-plan-card">
            <div className="download-file"><div><strong>{downloadPlan.fileName}</strong><span>{formatBytes(downloadPlan.expectedSize)} · {downloadPlan.architecture}</span></div><span className="apt-index-badge">APT 索引</span></div>
            <dl><div><dt>版本</dt><dd>{downloadPlan.version}</dd></div><div><dt>SHA-256</dt><dd title={downloadPlan.expectedSha256}>{downloadPlan.expectedSha256}</dd></div><div><dt>缓存位置</dt><dd title={downloadPlan.targetPath}>{downloadPlan.targetPath}</dd></div></dl>
          </div>}
          {downloadResult?.verified && <div className="download-success"><span>✓</span><div><strong>安装包校验通过</strong><p>{downloadResult.reusedExistingFile ? "已复用并重新验证缓存文件" : "文件已安全写入应用缓存"}</p></div></div>}
          {downloadError && <div className="inline-error">{downloadError}</div>}
          <button className="download-button" disabled={!downloadPlan || details.updateState !== "updateAvailable" || downloading} onClick={onDownload}>
            {downloading ? <><span className="spin">↻</span>正在下载并校验…</> : details.updateState === "upToDate" ? "已是最新版本，无需下载" : details.updateState === "unknown" ? "候选版本尚未确认" : "下载并校验官方 .deb"}
          </button>
          <p className="download-safety">下载不会请求 root 权限；文件仅在全部校验通过后进入缓存。</p>
        </section>
        <section className="detail-section"><h3>操作计划</h3><div className="plan-list">{details.operationPlan.map((step) => <div className={step.state} key={step.order}><span>{step.state === "complete" ? "✓" : step.order}</span><div><strong>{step.action}</strong><p>{step.detail}</p></div></div>)}</div></section>
        <section className="detail-section final-plan-section"><h3>最终更新计划</h3>
          {downloadResult?.verified && downloadPlan ? <>
            <div className="final-plan-card"><dl>
              <div><dt>动作</dt><dd>install-verified-deb（dry-run）</dd></div>
              <div><dt>包</dt><dd>code · amd64</dd></div>
              <div><dt>版本</dt><dd>{details.installedVersion} → {downloadPlan.version}</dd></div>
              <div><dt>SHA-256</dt><dd title={downloadResult.actualSha256}>{downloadResult.actualSha256}</dd></div>
            </dl></div>
            {!operationPlan && <label className="plan-confirmation"><input type="checkbox" checked={operationConfirmed} onChange={(event) => onConfirmationChange(event.target.checked)}/><span>我已核对动作、包名、版本、架构和 SHA-256，同意生成 15 分钟内有效的不可变计划。</span></label>}
            {operationError && <div className="inline-error">{operationError}</div>}
            {!operationPlan && <button className="download-button" disabled={!operationConfirmed || operationLoading} onClick={onCreateOperationPlan}>{operationLoading ? "正在锁定计划…" : "确认并锁定操作计划"}</button>}
            {operationPlan && <div className="immutable-plan"><strong>计划已锁定</strong><span>ID：{operationPlan.plan.planId}</span><span>有效至：{new Date(operationPlan.plan.payload.expiresAtUnixSeconds * 1000).toLocaleTimeString("zh-CN")}</span></div>}
            {dryRunError && <div className="inline-error">{dryRunError}</div>}
            {operationPlan && !dryRunResult && <button className="dry-run-button" disabled={dryRunRunning} onClick={onDryRun}>{dryRunRunning ? "正在特权环境复核…" : "请求授权并执行 dry-run"}</button>}
            {dryRunResult && <div className="dry-run-success"><strong>✓ dry-run 全部通过</strong><span>helper 已独立复核计划与安装包；本次未修改系统。</span></div>}
          </> : <p className="final-plan-unavailable">{details.updateState === "upToDate" ? "当前已是最新版本，不会生成重装或降级计划。" : "完成官方 .deb 下载与校验后，可在此核对并锁定计划。"}</p>}
        </section>
        <div className="readonly-note">当前仅支持 dry-run；不会向 dpkg 发出安装命令，也绝不允许降级。</div>
      </div>}
    </aside>
  </div>;
}

export default function App() {
  const [result, setResult] = useState<ScanResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [vscodeDetails, setVscodeDetails] = useState<VscodeDetails | null>(null);
  const [detailsLoading, setDetailsLoading] = useState(false);
  const [detailsError, setDetailsError] = useState<string | null>(null);
  const [downloadPlan, setDownloadPlan] = useState<VscodeDownloadPlan | null>(null);
  const [planError, setPlanError] = useState<string | null>(null);
  const [downloading, setDownloading] = useState(false);
  const [downloadResult, setDownloadResult] = useState<VscodeDownloadResult | null>(null);
  const [downloadError, setDownloadError] = useState<string | null>(null);
  const [operationConfirmed, setOperationConfirmed] = useState(false);
  const [operationPlan, setOperationPlan] = useState<OperationPlanArtifact | null>(null);
  const [operationLoading, setOperationLoading] = useState(false);
  const [operationError, setOperationError] = useState<string | null>(null);
  const [dryRunRunning, setDryRunRunning] = useState(false);
  const [dryRunResult, setDryRunResult] = useState<DryRunReport | null>(null);
  const [dryRunError, setDryRunError] = useState<string | null>(null);
  const [pendingLocalDeb, setPendingLocalDeb] = useState<LocalDebInspection | null>(null);
  const [pendingLocalDebError, setPendingLocalDebError] = useState<string | null>(null);
  const [wechatOpen, setWechatOpen] = useState(false);
  const [wechatDetails, setWechatDetails] = useState<WechatDetails | null>(null);
  const [wechatLoading, setWechatLoading] = useState(false);
  const [wechatError, setWechatError] = useState<string | null>(null);

  const refresh = async () => {
    setScanning(true); setError(null);
    try { setResult(await scanPackages()); } catch (reason) { setError(String(reason)); } finally { setScanning(false); }
  };
  useEffect(() => {
    void refresh();
    void getPendingLocalDeb().then(setPendingLocalDeb).catch((reason) => setPendingLocalDebError(String(reason)));
  }, []);
  const openVscode = async () => {
    setDetailsOpen(true); setDetailsLoading(true); setVscodeDetails(null); setDetailsError(null); setDownloadPlan(null); setPlanError(null); setDownloadResult(null); setDownloadError(null); setOperationConfirmed(false); setOperationPlan(null); setOperationError(null); setDryRunResult(null); setDryRunError(null);
    const [detailsOutcome, planOutcome] = await Promise.allSettled([getVscodeDetails(), getVscodeDownloadPlan()]);
    if (detailsOutcome.status === "fulfilled") setVscodeDetails(detailsOutcome.value); else setDetailsError(String(detailsOutcome.reason));
    if (planOutcome.status === "fulfilled") setDownloadPlan(planOutcome.value); else setPlanError(String(planOutcome.reason));
    setDetailsLoading(false);
  };
  const downloadVscode = async () => {
    setDownloading(true); setDownloadError(null); setDownloadResult(null);
    try { setDownloadResult(await downloadVscodePackage()); } catch (reason) { setDownloadError(String(reason)); } finally { setDownloading(false); }
  };
  const openWechat = async () => {
    setWechatOpen(true); setWechatLoading(true); setWechatDetails(null); setWechatError(null);
    try {
      const details = await getWechatDetails();
      setWechatDetails(details);
      setResult((current) => current ? { ...current, packages: current.packages.map((item) => item.packageName === "wechat" ? { ...item, candidateVersion: details.packageVersion, updateState: details.updateState } : item) } : current);
    } catch (reason) { setWechatError(String(reason)); } finally { setWechatLoading(false); }
  };
  const createOperationPlan = async () => {
    if (!operationConfirmed) return;
    setOperationLoading(true); setOperationError(null);
    try { setOperationPlan(await createVscodeOperationPlan()); } catch (reason) { setOperationError(String(reason)); } finally { setOperationLoading(false); }
  };
  const runDryRun = async () => {
    if (!operationPlan) return;
    setDryRunRunning(true); setDryRunError(null);
    try { setDryRunResult(await runVscodeOperationDryRun(operationPlan.plan.planId)); } catch (reason) { setDryRunError(String(reason)); } finally { setDryRunRunning(false); }
  };
  const stats = useMemo(() => summarizePackages(result?.packages ?? []), [result]);
  const visible = useMemo(() => (result?.packages ?? []).filter((item) => {
    const textMatch = `${item.displayName} ${item.vendor} ${item.packageName}`.toLowerCase().includes(query.toLowerCase());
    const filterMatch = filter === "all" || (filter === "updates" && item.updateState === "updateAvailable") || (filter === "local" && item.sourceKind === "localPackage");
    return textMatch && filterMatch;
  }), [filter, query, result]);

  return <div className="app-shell">
    <aside className="sidebar">
      <div className="brand"><span className="brand-mark">U</span><strong>UManager</strong></div>
      <nav aria-label="主导航">
        <button className="nav-item active"><Icon name="apps"/>软件</button>
        <button className="nav-item" disabled><Icon name="source"/>软件源<span className="later">稍后</span></button>
        <button className="nav-item" disabled><Icon name="history"/>操作记录<span className="later">稍后</span></button>
      </nav>
      <div className="sidebar-spacer"/>
      <div className="safety-card"><Icon name="shield"/><div><strong>只读模式</strong><span>不会更改系统</span></div></div>
      <button className="nav-item settings" disabled><Icon name="settings"/>设置</button>
      <div className="version-label">UManager 0.1.0</div>
    </aside>

    <main className="workspace">
      <header className="workspace-header"><div><h1>软件</h1><p>来自厂商官网的已安装应用</p></div><button className="primary-button" onClick={() => void refresh()} disabled={scanning}><span className={scanning ? "spin" : ""}>↻</span>{scanning ? "扫描中…" : "检查更新"}</button></header>
      <section className="summary-strip" aria-label="扫描摘要">
        <div><span className="summary-number">{stats.total}</span><span>个受管软件</span></div><i/>
        <div><span className="summary-number">{stats.repositories}</span><span>个官方软件源</span></div><i/>
        <div className={stats.updates ? "has-updates" : ""}><span className="summary-number">{stats.updates}</span><span>个可用更新</span></div>
        {stats.pendingRepositoryChecks > 0 && <><i/><div className="needs-confirm"><span className="summary-number">{stats.pendingRepositoryChecks}</span><span>项待确认</span></div></>}
        {result && <time>上次扫描：{new Date(result.scannedAtUnixSeconds * 1000).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })}</time>}
      </section>
      <section className="software-panel">
        {pendingLocalDebError && <div className="message error"><strong>无法打开本地 .deb</strong><span>{pendingLocalDebError}</span></div>}
        <div className="panel-toolbar"><div className="filter-tabs">
          <button className={filter === "all" ? "active" : ""} onClick={() => setFilter("all")}>全部</button>
          <button className={filter === "updates" ? "active" : ""} onClick={() => setFilter("updates")}>可更新 {stats.updates > 0 && <b>{stats.updates}</b>}</button>
          <button className={filter === "local" ? "active" : ""} onClick={() => setFilter("local")}>本地安装</button>
        </div><label className="search-box"><Icon name="search"/><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索软件"/></label></div>
        <div className="table-head"><span>软件</span><span>版本</span><span>来源</span><span>状态</span></div>
        {error && <div className="message error"><strong>无法读取软件信息</strong><span>{error}</span></div>}
        {result?.warnings.map((warning) => <div className="message" key={warning}>{warning}</div>)}
        {!result && !error && <div className="empty-state"><span className="loader"/><p>正在读取 dpkg 与 APT 缓存…</p></div>}
        {result && visible.length === 0 && <div className="empty-state"><p>没有符合条件的软件</p></div>}
        <div className="package-list">{visible.map((item) => <PackageRow item={item} onOpen={item.packageName === "code" ? () => void openVscode() : item.packageName === "wechat" ? () => void openWechat() : undefined} key={item.packageName}/>)}</div>
      </section>
    </main>
    {detailsOpen && <VscodeDrawer details={vscodeDetails} loading={detailsLoading} error={detailsError} downloadPlan={downloadPlan} planError={planError} downloading={downloading} downloadResult={downloadResult} downloadError={downloadError} operationConfirmed={operationConfirmed} operationPlan={operationPlan} operationLoading={operationLoading} operationError={operationError} dryRunRunning={dryRunRunning} dryRunResult={dryRunResult} dryRunError={dryRunError} onConfirmationChange={setOperationConfirmed} onCreateOperationPlan={() => void createOperationPlan()} onDryRun={() => void runDryRun()} onDownload={() => void downloadVscode()} onClose={() => setDetailsOpen(false)}/>}
    {wechatOpen && <WechatDrawer details={wechatDetails} loading={wechatLoading} error={wechatError} onClose={() => setWechatOpen(false)}/>}
    {pendingLocalDeb && <LocalDebDialog initial={pendingLocalDeb} onClose={() => setPendingLocalDeb(null)} onInstalled={() => void refresh()}/>}
  </div>;
}
