import { useEffect, useMemo, useRef, useState } from "react";
import type { Dispatch, ReactNode, SetStateAction } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeRaw from "rehype-raw";
import rehypeSanitize, { defaultSchema } from "rehype-sanitize";
import { clearClipboardHistory, copyClipboardEntry, createLocalDebOperationPlan, createOperationPlan, createRemovalOperationPlan, deleteClipboardEntry, downloadPackage, dragClipboardImage, getAppIcon, getCategories, getApplicationDetails, getClipboardHistoryRevision, getClipboardHotkey, getClipboardImage, getDevReleases, getDevToolchains, getDevToolchainState, getDevTools, getDevToolState, getDownloadPlan, getFeedStatus, getInstallableApplications, getInstallationInfo, getLlmSettings, getNetworkSettings, getPendingLocalDeb, getSessionInfo, getSoftwareCatalog, hideClipboardPanel, importPendingLocalDeb, installDevTool, installDevVersion, installLocalDeb, installPackage, launchApplication, listClipboardHistory, listScripts, notifyDownloadComplete, onClipboardHistoryChanged, openExternalUrl, refreshFeed, removeManagedPackage, restartApp, runLocalDebDryRun, runOperationDryRun, runRemovalDryRun, scanPackages, setClipboardEntryPinned, setClipboardHotkey, setDevDefaultVersion, setLlmSettings, setNetworkSettings, runScript, stopScript, testLlmConnection, translateChangelog, uninstallDevTool, uninstallDevVersion, updateDevTool } from "./api";
import type { ApplicationDetails, CatalogApplication, CategoryCatalog, ClipboardEntry, DevOperationProgress, DevOperationReport, DevRelease, DevTool, DevToolchain, DevToolchainState, DevToolProgress, DevToolReport, DevToolState, DownloadPlan, DownloadProgress, DownloadResult, DryRunReport, FeedStatus, InstallableApplication, InstallationInfo, LlmSettings, LocalDebInspection, ManagedPackage, NetworkSettings, OperationExecutionReport, OperationPlanArtifact, OperationProgressEvent, RemovalExecutionReport, RemovalPlanArtifact, ScanResult, ScriptAction, ScriptDefinition, ScriptProgressEvent, SessionInfo, UpdateState } from "./types";
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
import dshIcon from "./assets/app-icons/dsh.svg?no-inline";
import hermesIcon from "./assets/app-icons/hermes.png";
import feishuIcon from "./assets/app-icons/feishu.png";
import wpsIcon from "./assets/app-icons/wps.svg?no-inline";

type Filter = "all" | "installed" | "updates" | "installable";
type Page = "installed" | "updates" | "dev" | "scripts" | "clipboard" | "settings";
const sourceText = { officialRepository: "官方 APT 仓库", officialWebsite: "官网直连", localPackage: "本地 .deb" } as const;
const iconAssets: Record<string, string> = { vscode: vscodeIcon, "google-chrome": chromeIcon, chatgpt: chatgptIcon, flclash: flclashIcon, wechat: wechatIcon, wemeet: wemeetIcon, wps: wpsIcon, nodejs: nodejsIcon, rust: rustIcon, claude: claudeIcon, opencode: opencodeIcon, pi: piIcon, codex: codexIcon, dsh: dshIcon, hermes: hermesIcon, "github-cli": githubCliIcon, feishu: feishuIcon };
const fallbackIconKey: Record<string, string> = { code: "vscode", "google-chrome-stable": "google-chrome", chatgpt: "chatgpt", flclash: "flclash", wechat: "wechat", wemeet: "wemeet", "wps-office": "wps" };
const fallbackColors: Record<string, string> = { code: "#2b78bd", "google-chrome-stable": "#4285f4", chatgpt: "#171918", flclash: "#7c5ce5", wechat: "#22ad38", wemeet: "#2878ff" };

let catalogByPackage: Record<string, CatalogApplication> = {};

function clipboardPanelMode(): boolean {
  if (!("__TAURI_INTERNALS__" in window)) return false;
  try {
    return getCurrentWebviewWindow().label === "clipboard-panel";
  } catch {
    return false;
  }
}

function appearance(packageName: string) {
  const entry = catalogByPackage[packageName];
  return {
    iconKey: entry?.icon ?? fallbackIconKey[packageName] ?? null,
    accentColor: entry?.accentColor ?? fallbackColors[packageName] ?? "#555",
    iconUrl: entry?.iconUrl ?? null,
    iconSha256: entry?.iconSha256 ?? null,
  };
}

function Icon({ name }: { name: "apps" | "source" | "history" | "update" | "back" | "clipboard" | "settings" | "search" | "shield" | "dev" | "script" | "external" }) {
  const paths = {
    apps: <><rect x="3" y="3" width="7" height="7" rx="2"/><rect x="14" y="3" width="7" height="7" rx="2"/><rect x="3" y="14" width="7" height="7" rx="2"/><rect x="14" y="14" width="7" height="7" rx="2"/></>,
    source: <><path d="M4 7h16M6 3h12l2 4-2 4H6L4 7l2-4Z"/><path d="M7 11v10m10-10v10M4 21h16"/></>,
    history: <><path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5M12 7v5l3 2"/></>,
    update: <><path d="M12 3v11"/><path d="m7 10 5 5 5-5"/><path d="M5 21h14"/></>,
    back: <><path d="M15 5l-7 7 7 7"/></>,
    clipboard: <><rect x="8" y="2" width="8" height="4" rx="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/><path d="M9 12h6M9 16h4"/></>,
    settings: <><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.9l.1.1-2.8 2.8-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.6v.2h-4V21a1.7 1.7 0 0 0-1-1.6 1.7 1.7 0 0 0-1.9.3l-.1.1L4.2 17l.1-.1a1.7 1.7 0 0 0 .3-1.9A1.7 1.7 0 0 0 3 14H2.8v-4H3a1.7 1.7 0 0 0 1.6-1 1.7 1.7 0 0 0-.3-1.9L4.2 7 7 4.2l.1.1a1.7 1.7 0 0 0 1.9.3A1.7 1.7 0 0 0 10 3V2.8h4V3a1.7 1.7 0 0 0 1 1.6 1.7 1.7 0 0 0 1.9-.3l.1-.1L19.8 7l-.1.1a1.7 1.7 0 0 0-.3 1.9 1.7 1.7 0 0 0 1.6 1h.2v4H21a1.7 1.7 0 0 0-1.6 1Z"/></>,
    search: <><circle cx="11" cy="11" r="7"/><path d="m20 20-4-4"/></>,
    shield: <><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10Z"/><path d="m9 12 2 2 4-4"/></>,
    dev: <><path d="M8 6 3 12l5 6M16 6l5 6-5 6M14 4l-4 16"/></>,
    script: <><rect x="3" y="4" width="18" height="16" rx="2"/><path d="m7 9 3 3-3 3M13 15h4"/></>,
    external: <><path d="M14 4h6v6"/><path d="M20 4 10 14"/><path d="M20 13v5a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h5"/></>,
  };
  return <svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true">{paths[name]}</svg>;
}

function useModalFocus<T extends HTMLElement>(onClose: () => void, canClose: boolean) {
  const ref = useRef<T | null>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  const canCloseRef = useRef(canClose);
  canCloseRef.current = canClose;
  useEffect(() => {
    const node = ref.current;
    if (!node) return;
    const previouslyFocused = document.activeElement as HTMLElement | null;
    const focusables = () => Array.from(node.querySelectorAll<HTMLElement>(
      'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
    )).filter((element) => !element.hasAttribute("disabled") && element.getAttribute("aria-hidden") !== "true");
    (focusables()[0] ?? node).focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        if (canCloseRef.current) onCloseRef.current();
        return;
      }
      if (event.key !== "Tab") return;
      const list = focusables();
      if (list.length === 0) { event.preventDefault(); return; }
      const first = list[0];
      const last = list[list.length - 1];
      const active = document.activeElement;
      if (event.shiftKey && (active === first || active === node)) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && active === last) {
        event.preventDefault();
        first.focus();
      }
    };
    node.addEventListener("keydown", onKeyDown);
    return () => {
      node.removeEventListener("keydown", onKeyDown);
      previouslyFocused?.focus?.();
    };
  }, []);
  return ref;
}

function DetailShell({ label, icon, title, subtitle, description, action, canClose, onClose, children }: {
  label: string;
  icon: ReactNode;
  title: string;
  subtitle: string;
  description?: string | null;
  action?: ReactNode;
  canClose: boolean;
  onClose: () => void;
  children: ReactNode;
}) {
  const focusRef = useModalFocus<HTMLDivElement>(onClose, canClose);
  return <div className="drawer-layer" onMouseDown={(event) => { if (event.currentTarget === event.target && canClose) onClose(); }}>
    <div className="detail-drawer" role="dialog" aria-modal="true" aria-label={label} ref={focusRef} tabIndex={-1}>
      <div className="detail-topbar"><button className="back-button" onClick={onClose} disabled={!canClose}><Icon name="back"/><span>返回</span></button></div>
      <div className="detail-page-body">
        <header className="detail-hero">
          <div className="detail-hero-icon">{icon}</div>
          <div className="detail-hero-main">
            <h1 className="detail-hero-title">{title}</h1>
            <span className="detail-hero-subtitle">{subtitle}</span>
            {description && <p className="detail-hero-desc">{description}</p>}
          </div>
          {action && <div className="detail-hero-action">{action}</div>}
        </header>
        <div className="detail-page-content">{children}</div>
      </div>
    </div>
  </div>;
}

function DependencyGapWarning({ missing }: { missing: string[] }) {
  if (missing.length === 0) return null;
  return <div className="dependency-warning"><strong>⚠ 检测到缺少依赖</strong><span>UManager 用 <code>dpkg --install</code> 安装，不会自动补装依赖；缺少以下包可能导致安装失败：</span><ul>{missing.map((item, index) => <li key={`${index}-${item}`}><code>{item}</code></li>)}</ul><p>可先在终端执行 <code>sudo apt-get install -f</code> 或手动安装上述包后再继续。</p></div>;
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

// 只有带 .desktop 启动项的图形应用才提供「打开」；CLI 工具（category === "cli"）没有可启动入口。
// UManager 自身不提供「打开」（它已经在运行），更新入口在更新抽屉里以「重启 UManager」呈现。
function isLaunchable(packageName: string) {
  if (packageName === "u-manager") return false;
  const entry = catalogByPackage[packageName];
  if (!entry) return false;
  return entry.category !== "cli";
}

function formatBytes(bytes: number) {
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MiB`;
}

function formatSpeed(bytesPerSecond: number) {
  if (bytesPerSecond >= 1024 * 1024) return `${(bytesPerSecond / 1024 / 1024).toFixed(1)} MiB/s`;
  return `${(bytesPerSecond / 1024).toFixed(0)} KiB/s`;
}

function formatUpdatedAt(unixSeconds: number | null | undefined) {
  if (unixSeconds == null) return null;
  return new Date(unixSeconds * 1000).toLocaleString("zh-CN", { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" });
}

// Release notes are Markdown from vendor GitHub release bodies. Render them as
// sanitized HTML: strip any element that loads external resources (images,
// media, iframes — the CSP blocks those anyway) but keep headings, lists,
// code, tables and links. Links open in the default browser via gio, never by
// navigating the app webview.
const releaseNotesSchema = {
  ...defaultSchema,
  tagNames: (defaultSchema.tagNames ?? []).filter(
    (name) => !["img", "picture", "source", "track", "video", "audio", "iframe", "embed", "object", "svg", "math", "canvas"].includes(name),
  ),
};

function ChangelogLink({ url }: { url: string | null | undefined }) {
  if (!url) return null;
  return <p className="release-notes-link">完整更新记录：<a href={url} onClick={(event) => { event.preventDefault(); void openExternalUrl(url); }}>{url}</a></p>;
}

// Lightweight heuristic: a changelog is offered translation when it contains
// Latin text but no CJK characters (i.e. it reads as English to a zh-CN user).
function looksEnglish(text: string): boolean {
  if (!text) return false;
  const hasCjk = /[\u3040-\u30ff\u3400-\u4dbf\u4e00-\u9fff\uac00-\ud7af]/.test(text);
  const hasLatin = /[A-Za-z]{3,}/.test(text);
  return hasLatin && !hasCjk;
}

function useChangelogTranslation(notes: string | null | undefined) {
  const [translated, setTranslated] = useState<string | null>(null);
  const [streaming, setStreaming] = useState<string | null>(null);
  const [showTranslated, setShowTranslated] = useState(false);
  const [translating, setTranslating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [llmEnabled, setLlmEnabled] = useState(false);
  const requestSeq = useRef(0);
  const english = looksEnglish(notes ?? "");

  useEffect(() => {
    if (!english) { setLlmEnabled(false); return; }
    let cancelled = false;
    getLlmSettings()
      .then((settings) => { if (!cancelled) setLlmEnabled(settings.enabled); })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [english]);

  const toggle = async () => {
    if (translating || !notes) return;
    if (showTranslated) { setShowTranslated(false); return; }
    if (translated) { setShowTranslated(true); return; }
    setTranslating(true); setError(null); setStreaming(null);
    requestSeq.current += 1;
    const requestId = `translate-${Date.now()}-${requestSeq.current}`;
    try {
      const result = await translateChangelog(notes, requestId, (delta) => {
        setStreaming((previous) => (previous ?? "") + delta);
      });
      setTranslated(result);
      setShowTranslated(true);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setTranslating(false);
    }
  };

  const content = translating
    ? (streaming ?? "")
    : (showTranslated && translated ? translated : notes ?? "");

  return {
    content,
    pending: translating && !streaming,
    canTranslate: english && llmEnabled,
    translating,
    showTranslated,
    error,
    toggle,
  };
}

function ChangelogTranslateButton({ translation }: { translation: ReturnType<typeof useChangelogTranslation> }) {
  if (!translation.canTranslate) return null;
  return <button className="translate-toggle" onClick={() => void translation.toggle()} disabled={translation.translating}>
    {translation.translating ? "翻译中…" : (translation.showTranslated ? "查看原文" : "翻译为中文")}
  </button>;
}

function ChangelogMarkdown({ content, pending }: { content: string; pending: boolean }) {
  // No changelog body: render nothing (an empty `.release-notes` box would show
  // as a stray white bar). During a translation request we still render the box
  // so the "正在请求翻译…" placeholder is visible.
  if (!pending && !content.trim()) return null;
  return <div className="release-notes">
    {pending && <p className="release-notes-pending">正在请求翻译…</p>}
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      rehypePlugins={[rehypeRaw, [rehypeSanitize, releaseNotesSchema]]}
      components={{
        a: ({ href, children }) => (
          <a href={href} onClick={(event) => { event.preventDefault(); if (href) void openExternalUrl(href); }}>{children}</a>
        ),
      }}
    >
      {content}
    </ReactMarkdown>
  </div>;
}

function ReleaseNotes({ notes, url }: { notes: string | null | undefined; url: string | null | undefined }) {
  const translation = useChangelogTranslation(notes);
  if (!notes && !url) return null;
  return <section className="detail-section release-notes-section">
    <div className="release-notes-head">
      <h3>版本更新记录</h3>
      <ChangelogTranslateButton translation={translation}/>
    </div>
    {translation.error && <p className="changelog-translate-error">{translation.error}</p>}
    <ChangelogMarkdown content={translation.content} pending={translation.pending}/>
    <ChangelogLink url={url}/>
  </section>;
}

function WhatsNew({ version, seconds, notes, url }: { version: string | null; seconds: number | null | undefined; notes: string | null | undefined; url: string | null | undefined }) {
  const translation = useChangelogTranslation(notes);
  if (!version && !notes && !url) return null;
  const dateText = seconds != null ? formatUpdatedAt(seconds) : null;
  return <section className="detail-section whats-new-section">
    <h3>新内容</h3>
    <div className="whats-new-head">
      <div className="whats-new-meta">
        {version && <strong className="whats-new-version">版本 {version}</strong>}
        {dateText && <span className="whats-new-date">{dateText}</span>}
      </div>
      <ChangelogTranslateButton translation={translation}/>
    </div>
    {translation.error && <p className="changelog-translate-error">{translation.error}</p>}
    <ChangelogMarkdown content={translation.content} pending={translation.pending}/>
    <ChangelogLink url={url}/>
  </section>;
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
  const focusRef = useModalFocus<HTMLElement>(onClose, busy === null);
  return <div className="local-deb-layer">
    <section className="local-deb-dialog" role="dialog" aria-modal="true" aria-label="安装本地 Debian 包" ref={focusRef} tabIndex={-1}>
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
        {plan && <DependencyGapWarning missing={plan.missingDependencies}/>}
        {plan && !dryRun && <button className="dry-run-button" disabled={busy !== null} onClick={() => void run("dry-run", () => runLocalDebDryRun(plan!.plan.planId), setDryRun)}>{busy === "dry-run" ? "正在特权环境复核…" : "授权并执行安装前 dry-run"}</button>}
        {plan && dryRun && !installed && <><div className="dry-run-success"><strong>✓ 安装前复核通过</strong><span>helper 已独立复核路径、包名、版本、架构、大小和 SHA-256。</span></div><button className="install-local-button" disabled={busy !== null} onClick={() => void run("install", () => { setProgressEvents([]); return installLocalDeb(plan!.plan.planId, (event) => appendProgress(setProgressEvents, event)); }, (value) => { setInstalled(value); onInstalled(); })}>{busy === "install" ? "正在安装…" : `授权并安装 ${inspected.packageName}`}</button></>}
        <OperationLogPanel events={progressEvents} running={busy === "install"}/>
        {installed && <div className="dry-run-success"><strong>✓ 安装命令已成功完成</strong><span>已重新扫描；若该软件已有 UManager 厂商适配，它会出现在受管列表中。</span></div>}
      </div>
    </section>
  </div>;
}

function RemovalDialog({ item, onClose, onRemoved }: { item: ManagedPackage; onClose: () => void; onRemoved: () => void }) {
  const isSelfRemoval = item.packageName === "u-manager";
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

  const focusRef = useModalFocus<HTMLElement>(onClose, busy === null);
  return <div className="local-deb-layer">
    <section className="local-deb-dialog removal-dialog" role="dialog" aria-modal="true" aria-label={`卸载 ${item.displayName}`} ref={focusRef} tabIndex={-1}>
      <header><div>{isSelfRemoval ? <span className="brand-mark">U</span> : <AppMark item={item}/>}<div><h2>卸载 {item.displayName}</h2><p>{item.vendor} · {item.packageName}</p></div></div><button className="close-button" onClick={onClose} disabled={busy !== null} aria-label="关闭">×</button></header>
      <div className="local-deb-content">
        <div className="removal-warning"><strong>{isSelfRemoval ? "这会移除 UManager 程序本身" : "这会从系统中移除该软件"}</strong><span>{isSelfRemoval ? "只执行固定的 /usr/bin/dpkg --remove u-manager；不会请求 purge，也不会删除 UManager 缓存、下载的软件包、操作计划或你的个人文件。" : "UManager 不会请求 purge、自动删除依赖或直接删除你的个人目录；但 Debian 包自带的卸载脚本仍会以 root 权限运行。"}</span></div>
        <dl className="local-deb-facts">
          <div><dt>包名</dt><dd>{item.packageName}</dd></div><div><dt>动作</dt><dd>{isSelfRemoval ? "remove-umanager" : "dpkg --remove"}</dd></div>
          <div><dt>已安装</dt><dd>{item.installedVersion}</dd></div><div><dt>架构</dt><dd>{item.architecture}</dd></div>
          <div className="wide"><dt>{isSelfRemoval ? "执行后" : "范围"}</dt><dd>{isSelfRemoval ? "当前窗口可继续显示结果；关闭后 UManager 将无法再次启动" : `仅移除白名单中的 ${item.packageName} 包`}</dd></div>
        </dl>
        {error && <div className="inline-error removal-error">{error}</div>}
        {!plan && <><label className="plan-confirmation removal-confirmation"><input type="checkbox" checked={confirmed} onChange={(event) => setConfirmed(event.target.checked)}/><span>{isSelfRemoval ? "我确认要卸载当前 `.deb` 安装的 UManager，并理解保留的缓存数据需要日后手动清理。" : "我已核对软件、包名、版本和架构，并理解卸载脚本将以 root 权限运行。"}</span></label><button className="download-button" disabled={!confirmed || busy !== null} onClick={() => void run("plan", () => createRemovalOperationPlan(item.packageName), setPlan)}>{busy === "plan" ? "正在锁定计划…" : isSelfRemoval ? "确认并锁定自卸载计划" : "确认并锁定卸载计划"}</button></>}
        {plan && <div className="immutable-plan"><strong>{isSelfRemoval ? "自卸载计划已锁定" : "卸载计划已锁定"}</strong><span>ID：{plan.plan.planId}</span><span>有效至：{new Date(plan.plan.payload.expiresAtUnixSeconds * 1000).toLocaleTimeString("zh-CN")}</span></div>}
        {plan && !dryRun && <button className="dry-run-button" disabled={busy !== null} onClick={() => void run("dry-run", () => runRemovalDryRun(plan.plan.planId), setDryRun)}>{busy === "dry-run" ? "正在特权环境复核…" : "授权并执行卸载前复核"}</button>}
        {dryRun && !removed && <><div className="dry-run-success"><strong>✓ 卸载前复核通过</strong><span>helper 已重新核对白名单、当前版本、架构和不可变计划；本次未修改系统。</span></div><button className="remove-confirm-button" disabled={busy !== null} onClick={() => void run("remove", () => { setProgressEvents([]); return removeManagedPackage(plan!.plan.planId, (event) => appendProgress(setProgressEvents, event)); }, (value) => { setRemoved(value); onRemoved(); })}>{busy === "remove" ? "正在卸载…" : `再次确认并卸载 ${item.displayName}`}</button></>}
        <OperationLogPanel events={progressEvents} running={busy === "remove"}/>
        {removed && <><div className="dry-run-success"><strong>{isSelfRemoval ? "✓ UManager 已从系统移除" : "✓ 卸载已完成"}</strong><span>{isSelfRemoval ? "关闭当前窗口后程序将退出；缓存和个人数据仍然保留。" : "dpkg 移除命令已成功结束，软件列表已重新扫描。"}</span></div><button className="download-button" onClick={onClose}>完成</button></>}
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

function LlmSettingsPanel() {
  const [settings, setSettings] = useState<LlmSettings | null>(null);
  const [enabled, setEnabled] = useState(false);
  const [baseUrl, setBaseUrl] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [model, setModel] = useState("");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [saved, setSaved] = useState(false);
  const [testResult, setTestResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setLoading(true); setError(null);
    getLlmSettings()
      .then((value) => {
        setSettings(value);
        setEnabled(value.enabled);
        setBaseUrl(value.baseUrl);
        setApiKey(value.apiKey);
        setModel(value.model);
      })
      .catch((reason) => setError(String(reason)))
      .finally(() => setLoading(false));
  }, []);

  const save = async () => {
    setSaving(true); setError(null); setSaved(false);
    try {
      const value = await setLlmSettings({ enabled, baseUrl, apiKey, model });
      setSettings(value);
      setEnabled(value.enabled);
      setBaseUrl(value.baseUrl);
      setApiKey(value.apiKey);
      setModel(value.model);
      setSaved(true);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSaving(false);
    }
  };

  const test = async () => {
    setTesting(true); setError(null); setTestResult(null);
    try {
      const reply = await testLlmConnection({ enabled: true, baseUrl, apiKey, model });
      setTestResult(`连接成功，模型返回：${reply}`);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setTesting(false);
    }
  };

  return <section className="settings-panel network-panel">
    <div className="settings-section-heading"><div><div><h2>LLM 翻译</h2><p>用你自己的 OpenAI 兼容服务，一键把英文更新日志翻译成中文</p></div></div><span className={`install-kind-badge ${enabled ? "" : "unknown"}`}>{enabled ? "已启用" : "未启用"}</span></div>
    {loading && <div className="settings-loading"><span className="loader"/><span>正在读取 LLM 翻译设置…</span></div>}
    {!loading && <>
      <div className="network-proxy-form">
        <label className="plan-confirmation proxy-toggle"><input type="checkbox" checked={enabled} onChange={(event) => { setEnabled(event.target.checked); setSaved(false); }} /><span>启用 LLM 翻译</span></label>
        <label className="proxy-url-field"><span>服务地址</span><input value={baseUrl} onChange={(event) => { setBaseUrl(event.target.value); setSaved(false); }} placeholder="https://api.deepseek.com/v1" disabled={!enabled} spellCheck={false} aria-label="LLM 服务地址"/></label>
        <label className="proxy-url-field"><span>API Key</span><input type="password" value={apiKey} onChange={(event) => { setApiKey(event.target.value); setSaved(false); }} placeholder="sk-…（本地服务可留空）" disabled={!enabled} spellCheck={false} aria-label="LLM API Key"/></label>
        <label className="proxy-url-field"><span>模型</span><input value={model} onChange={(event) => { setModel(event.target.value); setSaved(false); }} placeholder="deepseek-chat" disabled={!enabled} spellCheck={false} aria-label="LLM 模型名称"/></label>
        <p className="proxy-hint">兼容任何 OpenAI 风格接口：DeepSeek、Moonshot、OpenAI、Ollama 等。服务地址填 API 根路径（通常以 <code>/v1</code> 结尾），App 会请求 <code>/chat/completions</code>。API Key 仅保存在本机，只发给你填写的服务地址。</p>
        {error && <div className="inline-error">{error}</div>}
        {testResult && <div className="inline-note">{testResult}</div>}
        <div className="proxy-actions">
          <button className="primary-button" onClick={() => void save()} disabled={saving}>{saving ? "保存中…" : "保存 LLM 设置"}</button>
          <button className="secondary-button" onClick={() => void test()} disabled={testing || !enabled}>{testing ? "测试中…" : "测试连接"}</button>
          {saved && !error && <span className="proxy-saved">✓ 已保存</span>}
        </div>
      </div>
      <dl className="installation-facts network-facts">
        <div><dt>当前状态</dt><dd>{enabled ? "LLM 翻译已启用" : "未启用"}</dd></div>
        <div><dt>服务地址</dt><dd title={settings?.baseUrl ?? ""}>{settings?.baseUrl && settings.baseUrl.trim() ? settings.baseUrl : "未设置"}</dd></div>
        <div><dt>模型</dt><dd>{settings?.model && settings.model.trim() ? settings.model : "未设置"}</dd></div>
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

function SettingsPage({ info, loading, error, onRefresh }: { info: InstallationInfo | null; loading: boolean; error: string | null; onRefresh: () => void }) {
  const kindLabel = info ? { debianPackage: ".deb 安装版", portable: "便携版", development: "开发版" }[info.installationKind] : "检测中";
  const kindDescription = info?.installationKind === "debianPackage"
    ? "当前可执行文件属于系统中已安装的 Debian 软件包，可通过 UManager 安全卸载。"
    : info?.installationKind === "development"
      ? "当前从开发构建目录运行，不属于系统软件包；请直接停止开发进程。"
      : "当前可执行文件不属于已安装的 Debian 软件包；退出后可直接删除文件。";
  return <main className="workspace settings-workspace">
    <header className="workspace-header"><div><h1>设置</h1><p>查看 UManager 版本与安装形态</p></div><button className="secondary-button" onClick={onRefresh} disabled={loading}>{loading ? "检测中…" : "重新检测"}</button></header>
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
      </>}
    </section>
    <NetworkSettingsPanel/>
    <LlmSettingsPanel/>
    <FeedStatusPanel/>
  </main>;
}

function InfoPanel({ entries, homepage }: { entries: { label: string; value: string; mono?: boolean }[]; homepage?: string | null }) {
  return <section className="detail-section info-section">
    <h3>信息</h3>
    <dl className="info-panel">
      {entries.map((entry) => <div key={entry.label}><dt>{entry.label}</dt><dd className={entry.mono ? "mono" : ""} title={entry.value}>{entry.value}</dd></div>)}
      {homepage && <div><dt>开发者网站</dt><dd><a className="info-homepage-link" href={homepage} title={homepage} onClick={(event) => { event.preventDefault(); void openExternalUrl(homepage); }}>{homepage}<Icon name="external"/></a></dd></div>}
    </dl>
  </section>;
}

function CardDownloadRing({ progress }: { progress: DownloadProgress }) {
  const percent = progress.totalBytes > 0 ? Math.min(100, Math.round(progress.transferredBytes / progress.totalBytes * 100)) : 0;
  const label = progress.phase === "verifying" ? "校验中" : `${percent}%`;
  return <span className="card-download-ring" style={{ background: `conic-gradient(var(--accent) ${percent * 3.6}deg, rgba(0,0,0,0.06) 0deg)` }} role="progressbar" aria-label={label} aria-valuenow={percent} aria-valuemin={0} aria-valuemax={100} title={label}><span>{percent}</span></span>;
}

function HeroDownloadProgress({ progress }: { progress: DownloadProgress }) {
  const percent = progress.totalBytes > 0 ? Math.min(100, Math.round(progress.transferredBytes / progress.totalBytes * 100)) : 0;
  const label = progress.phase === "verifying" ? "正在校验…" : "正在下载…";
  return <div className="hero-download" role="progressbar" aria-label={label} aria-valuenow={percent} aria-valuemin={0} aria-valuemax={100}>
    <span className="hero-download-ring" style={{ background: `conic-gradient(var(--accent) ${percent * 3.6}deg, rgba(0,0,0,0.08) 0deg)` }}><span className="hero-download-ring-inner">{percent}%</span></span>
    <span className="hero-download-label">{label}</span>
    <span className="hero-download-stats">{formatBytes(progress.transferredBytes)} / {formatBytes(progress.totalBytes)}</span>
  </div>;
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

function UpdateDrawer({ item, download, onStartDownload, onClearDownload, onClose, onInstalled, onLaunch, onRemove }: { item: ManagedPackage; download: DownloadState | undefined; onStartDownload: (notify: { title: string; body: string }) => void; onClearDownload: () => void; onClose: () => void; onInstalled: () => void; onLaunch: () => void; onRemove: () => void }) {
  const applicationId = catalogByPackage[item.packageName]?.applicationId;
  const isSelfUpdate = item.packageName === "u-manager";
  const [details, setDetails] = useState<ApplicationDetails | null>(null);
  const [downloadPlan, setDownloadPlan] = useState<DownloadPlan | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [operationPlan, setOperationPlan] = useState<OperationPlanArtifact | null>(null);
  const [dryRun, setDryRun] = useState<OperationExecutionReport | null>(null);
  const [installed, setInstalled] = useState<OperationExecutionReport | null>(null);
  const [installPhase, setInstallPhase] = useState<"idle" | "planning" | "dry-run" | "installing" | "done">("idle");
  const [progressEvents, setProgressEvents] = useState<OperationProgressEvent[]>([]);

  useEffect(() => {
    setLoading(true); setDetails(null); setDownloadPlan(null); setError(null); setOperationPlan(null); setDryRun(null); setInstalled(null); setInstallPhase("idle"); setProgressEvents([]);
    if (!applicationId) { setLoading(false); setError(`软件源中未找到 ${item.packageName} 的适配策略`); return; }
    void Promise.allSettled([getApplicationDetails(applicationId), getDownloadPlan(applicationId)]).then(([detailsOutcome, planOutcome]) => {
      if (detailsOutcome.status === "fulfilled") setDetails(detailsOutcome.value); else setError(String(detailsOutcome.reason));
      if (planOutcome.status === "fulfilled") setDownloadPlan(planOutcome.value); else setError(String(planOutcome.reason));
      setLoading(false);
    });
  }, [applicationId, item.packageName]);

  const isWebsite = details?.sourceKind === "officialWebsite";
  const hasUpdate = details?.updateState === "updateAvailable";
  const downloadStatus = download?.status;
  const downloading = downloadStatus === "downloading" || downloadStatus === "verifying";
  const ready = downloadStatus === "ready";
  const downloadResult = download !== undefined && download.status === "ready" ? download.result : null;
  const downloadError = download !== undefined && download.status === "error" ? download.error : null;
  const installRunning = installPhase === "planning" || installPhase === "dry-run" || installPhase === "installing";
  const running = downloading || installRunning;
  const targetVersion = details?.candidateVersion ?? downloadPlan?.version ?? "";

  const install = async () => {
    setError(null);
    try {
      setInstallPhase("planning");
      const plan = await createOperationPlan(applicationId!);
      setOperationPlan(plan);
      setInstallPhase("dry-run");
      const dry = await runOperationDryRun(plan.plan.planId);
      setDryRun(dry);
      setInstallPhase("installing");
      const report = await installPackage(plan.plan.planId, (event) => appendProgress(setProgressEvents, event));
      setInstalled(report);
      onInstalled();
      onClearDownload();
      setInstallPhase("done");
    } catch (reason) {
      setError(String(reason));
      setInstallPhase("idle");
    }
  };

  const done = installPhase === "done";
  const launchable = isLaunchable(item.packageName);
  const removable = catalogByPackage[item.packageName]?.removable ?? false;
  const restartSelfUpdate = () => { setError(null); void restartApp().catch((reason) => setError(String(reason))); };
  let actionLabel: string;
  let actionDisabled: boolean;
  let onAction: () => void;
  if (done) {
    if (isSelfUpdate) { actionLabel = "重启 UManager"; actionDisabled = false; onAction = restartSelfUpdate; }
    else { actionLabel = "打开"; actionDisabled = !launchable; onAction = onLaunch; }
  }
  else if (installRunning) { actionLabel = installPhase === "planning" ? "准备中…" : installPhase === "dry-run" ? "复核中…" : "安装中…"; actionDisabled = true; onAction = () => {}; }
  else if (downloading) { actionLabel = "下载中…"; actionDisabled = true; onAction = () => {}; }
  else if (ready) { actionLabel = "安装"; actionDisabled = false; onAction = () => void install(); }
  else if (hasUpdate) { actionLabel = "更新"; actionDisabled = !downloadPlan; onAction = () => onStartDownload({ title: `${item.displayName} 下载完成`, body: `安装包已通过校验，回到 UManager 继续更新 ${targetVersion}（需系统授权）。` }); }
  else if (isSelfUpdate) { actionLabel = "已是最新"; actionDisabled = true; onAction = () => {}; }
  else { actionLabel = "打开"; actionDisabled = !launchable; onAction = onLaunch; }
  const heroAction = downloading && download?.progress
    ? <HeroDownloadProgress progress={download.progress}/>
    : <>
        <button className="hero-button" disabled={actionDisabled} onClick={onAction}>{actionLabel}</button>
        {removable && !installRunning && !downloading && <button className="ghost-link" onClick={onRemove}>卸载</button>}
      </>;

  return <DetailShell
    label={`${item.displayName} 详情`}
    icon={<AppMark item={item}/>}
    title={item.displayName}
    subtitle={`${item.vendor} · ${item.packageName}`}
    description={catalogByPackage[item.packageName]?.description}
    action={heroAction}
    canClose={!running}
    onClose={onClose}
  >
    {loading && <div className="drawer-loading"><span className="loader"/><p>正在读取官方源与安装包元数据…</p></div>}
    {error && !details && <div className="message error"><strong>无法检查 {item.displayName} 更新</strong><span>{error}</span></div>}
    {details && <div className="drawer-content">
      <div className={`trust-banner ${details.trusted ? "trusted" : "needsReview"}`}><Icon name="shield"/><div><strong>{details.trusted ? `来源验证通过 · ${sourceText[details.sourceKind] ?? details.sourceKind}` : "来源需要复核"}</strong><span>{isWebsite ? "官网、下载域名、Debian 包名和架构均符合软件源策略" : "包名、架构和官方仓库均符合软件源策略"}</span></div></div>
      <WhatsNew version={details.candidateVersion ?? downloadPlan?.version ?? null} seconds={details.versionUpdatedAtUnixSeconds} notes={details.releaseNotes} url={details.releaseNotesUrl}/>
      {downloadResult?.verified && <div className="download-success"><span>✓</span><div><strong>安装包校验通过</strong><p>大小、SHA-256、包名、版本和架构均通过；SHA-256：{downloadResult.actualSha256.slice(0, 16)}…</p></div></div>}
      {downloadError && <div className="inline-error">{downloadError}</div>}
      {error && details && <div className="inline-error">{error}</div>}

      <details className="tech-disclosure" open={downloading || ready || installRunning || installed !== null}>
        <summary><Icon name="shield"/>安全验证与安装详情<span className="tech-disclosure-hint">版本 · 来源 · 证据 · SHA-256 · 更新计划</span></summary>
        <section className="detail-section"><h3>版本</h3><div className="version-pair"><div><span>已安装</span><strong>{details.installedVersion ?? "未安装"}</strong></div><div><span>{isWebsite ? "官方包完整版本" : "候选版本"}</span><strong>{details.candidateVersion ?? downloadPlan?.version ?? "待解析"}</strong></div></div>{details.websiteVersion && <p className="version-source-note">官网/发布标签展示 {details.websiteVersion}；UManager 通过 HTTP Range 读取 `.deb` 控制信息，获得用于比较的完整版本。</p>}</section>
        <InfoPanel homepage={item.homepage} entries={[
          { label: "开发者", value: details.vendor },
          { label: "软件包", value: details.packageName, mono: true },
          { label: "架构", value: details.architecture, mono: true },
          { label: "下载大小", value: downloadPlan ? formatBytes(downloadPlan.expectedSize) : details.expectedSize != null ? formatBytes(details.expectedSize) : "—" },
          { label: "SHA-256", value: downloadPlan?.expectedSha256 ?? details.sha256 ?? "—", mono: true },
        ]}/>
        <section className="detail-section"><h3>官方来源</h3><div className="path-card"><div><span className={`source-dot ${details.sourceKind}`}/><strong>{isWebsite ? "厂商官方发布通道" : `${item.vendor} 官方 APT 仓库`}</strong></div><code title={details.sourceUrl}>{details.sourceUrl}</code><p>下载地址与所有 HTTPS 重定向都不能离开软件源中声明的允许域名。</p></div></section>
        <section className="detail-section"><h3>官方证据</h3><div className="evidence-list">{details.evidence.map((entry) => <div key={entry.label}><span className={entry.passed ? "check passed" : "check failed"}>{entry.passed ? "✓" : "!"}</span><div><strong>{entry.label}</strong><code>{entry.actual}</code></div></div>)}</div></section>
        <section className="detail-section download-section"><h3>官方安装包</h3>
          {downloadPlan && <DownloadCard plan={downloadPlan}/>}
          <p className="download-safety">下载不请求 root 权限；校验失败的文件不会进入缓存。下载完成后需再确认，才会请求系统授权并执行安装。</p>
        </section>
        {downloadResult?.verified && <section className="detail-section final-plan-section"><h3>最终更新计划</h3>
          <div className="final-plan-card"><dl><div><dt>动作</dt><dd>{isSelfUpdate ? "install-umanager" : isWebsite ? "install-verified-website-deb" : "install-verified-deb"}</dd></div><div><dt>包</dt><dd>{item.packageName} · {item.architecture}</dd></div><div><dt>版本</dt><dd>{item.installedVersion} → {downloadResult.version}</dd></div><div><dt>SHA-256</dt><dd title={downloadResult.actualSha256}>{downloadResult.actualSha256}</dd></div></dl></div>
          {operationPlan && <div className="immutable-plan"><strong>更新计划已锁定</strong><span>ID：{operationPlan.plan.planId}</span><span>有效至：{new Date(operationPlan.plan.payload.expiresAtUnixSeconds * 1000).toLocaleTimeString("zh-CN")}</span></div>}
          {operationPlan && <DependencyGapWarning missing={operationPlan.missingDependencies}/>}
          {dryRun && <div className="dry-run-success"><strong>✓ 特权环境复核通过</strong><span>helper 已重新核对来源、版本、架构和缓存中的安装包，本次未修改系统。</span></div>}
          <OperationLogPanel events={progressEvents} running={installPhase === "installing"}/>
          {installed && <div className="dry-run-success"><strong>✓ 更新安装完成</strong><span>固定参数 dpkg 安装命令已成功结束，软件列表已重新扫描。</span></div>}
        </section>}
      </details>
    </div>}
  </DetailShell>;
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

type DownloadState =
  | { status: "downloading" | "verifying"; progress: DownloadProgress }
  | { status: "ready"; result: DownloadResult }
  | { status: "error"; error: string };

function SoftwareRow({ item, category, progress, onOpen, onRemove, onLaunch }: { item: MergedSoftware; category: string; progress: DownloadProgress | null; onOpen: () => void; onRemove: () => void; onLaunch: () => void }) {
  const catalogApp = catalogByPackage[item.packageName];
  const autoInstallable = !!catalogApp && isAutoInstallable(item.packageName);
  const canOpen = item.installed ? autoInstallable : item.installAvailable;
  const updateAvailable = item.installed && item.updateState === "updateAvailable" && autoInstallable;
  const launchable = item.installed && isLaunchable(item.packageName);
  const downloading = progress !== null && (progress.phase === "downloading" || progress.phase === "verifying");
  const statusText = item.installed
    ? item.updateState === "updateAvailable" ? "可更新" : item.updateState === "upToDate" ? "已是最新" : "手动检查"
    : item.installAvailable ? "可安装" : "不可用";
  const statusClass = item.installed ? item.updateState : item.installAvailable ? "updateAvailable" : "unknown";
  return <article className={`app-card ${canOpen ? "supported" : ""}`} role={canOpen ? "button" : undefined} tabIndex={canOpen ? 0 : undefined} onClick={() => { if (canOpen) onOpen(); }} onKeyDown={(event) => { if (canOpen && (event.key === "Enter" || event.key === " ")) onOpen(); }}>
    <div className="app-card-top">
      <AppLogo packageName={item.packageName} displayName={item.displayName}/>
      <div className="app-card-actions">
        {downloading
          ? <CardDownloadRing progress={progress}/>
          : <>
            {!item.installed && item.installAvailable && <button className="get-button" onClick={(event) => { event.stopPropagation(); onOpen(); }} onKeyDown={(event) => event.stopPropagation()} aria-label={`获取 ${item.displayName}`}>获取</button>}
            {updateAvailable && <button className="get-button update" onClick={(event) => { event.stopPropagation(); onOpen(); }} onKeyDown={(event) => event.stopPropagation()} aria-label={`更新 ${item.displayName}`}>更新</button>}
            {!updateAvailable && launchable && <button className="get-button open" onClick={(event) => { event.stopPropagation(); onLaunch(); }} onKeyDown={(event) => event.stopPropagation()} aria-label={`打开 ${item.displayName}`}>打开</button>}
            {item.installed && item.removable && <button className="ghost-link" onClick={(event) => { event.stopPropagation(); onRemove(); }} onKeyDown={(event) => event.stopPropagation()} aria-label={`卸载 ${item.displayName}`}>卸载</button>}
          </>}
      </div>
    </div>
    <div className="app-card-body">
      <h3 className="app-card-name">{item.displayName}</h3>
      <span className="app-card-sub">{category}</span>
      {item.description && <p className="app-card-desc">{item.description}</p>}
    </div>
    <div className="app-card-footer">
      <span className={`status-badge ${statusClass}`}>{statusText}</span>
      <span className="app-card-version">{item.installed ? `v${item.installedVersion}` : item.candidateVersion ? `v${item.candidateVersion}` : item.architecture}</span>
    </div>
  </article>;
}

function InstallDrawer({ offer, download, onStartDownload, onClearDownload, onClose, onInstalled, onLaunch }: { offer: InstallableApplication; download: DownloadState | undefined; onStartDownload: (notify: { title: string; body: string }) => void; onClearDownload: () => void; onClose: () => void; onInstalled: () => void; onLaunch: () => void }) {
  const downloadPlan = offer.downloadPlan;
  const isWebsite = offer.sourceKind === "officialWebsite";
  const [operationPlan, setOperationPlan] = useState<OperationPlanArtifact | null>(null);
  const [dryRun, setDryRun] = useState<OperationExecutionReport | null>(null);
  const [installed, setInstalled] = useState<OperationExecutionReport | null>(null);
  const [installPhase, setInstallPhase] = useState<"idle" | "planning" | "dry-run" | "installing" | "done">("idle");
  const [error, setError] = useState<string | null>(null);
  const [progressEvents, setProgressEvents] = useState<OperationProgressEvent[]>([]);

  const downloadStatus = download?.status;
  const downloading = downloadStatus === "downloading" || downloadStatus === "verifying";
  const ready = downloadStatus === "ready";
  const downloadResult = download !== undefined && download.status === "ready" ? download.result : null;
  const downloadError = download !== undefined && download.status === "error" ? download.error : null;
  const installRunning = installPhase === "planning" || installPhase === "dry-run" || installPhase === "installing";
  const running = downloading || installRunning;

  const install = async () => {
    setError(null);
    try {
      setInstallPhase("planning");
      const plan = await createOperationPlan(offer.applicationId);
      setOperationPlan(plan);
      setInstallPhase("dry-run");
      const dry = await runOperationDryRun(plan.plan.planId);
      setDryRun(dry);
      setInstallPhase("installing");
      const report = await installPackage(plan.plan.planId, (event) => appendProgress(setProgressEvents, event));
      setInstalled(report);
      onInstalled();
      onClearDownload();
      setInstallPhase("done");
    } catch (reason) {
      setError(String(reason));
      setInstallPhase("idle");
    }
  };

  const done = installPhase === "done";
  const launchable = isLaunchable(offer.packageName);
  let actionLabel: string;
  let actionDisabled: boolean;
  let onAction: () => void;
  if (done) { actionLabel = "打开"; actionDisabled = !launchable; onAction = onLaunch; }
  else if (installRunning) { actionLabel = installPhase === "planning" ? "准备中…" : installPhase === "dry-run" ? "复核中…" : "安装中…"; actionDisabled = true; onAction = () => {}; }
  else if (downloading) { actionLabel = "下载中…"; actionDisabled = true; onAction = () => {}; }
  else if (ready) { actionLabel = "安装"; actionDisabled = false; onAction = () => void install(); }
  else { actionLabel = "获取"; actionDisabled = !downloadPlan; onAction = () => onStartDownload({ title: `${offer.displayName} 下载完成`, body: `安装包已通过校验，回到 UManager 继续安装（需系统授权）。` }); }
  const heroAction = downloading && download?.progress
    ? <HeroDownloadProgress progress={download.progress}/>
    : <button className="hero-button" disabled={actionDisabled} onClick={onAction}>{actionLabel}</button>;

  return <DetailShell
    label={`安装 ${offer.displayName}`}
    icon={<AppLogo packageName={offer.packageName} displayName={offer.displayName}/>}
    title={offer.displayName}
    subtitle={`${offer.vendor} · ${offer.packageName}`}
    description={offer.description}
    action={heroAction}
    canClose={!running}
    onClose={onClose}
  >
    <div className="drawer-content">
      <div className="trust-banner trusted"><Icon name="shield"/><div><strong>来源验证通过 · {sourceText[offer.sourceKind] ?? offer.sourceKind}</strong><span>{offer.packageName} · {offer.architecture} 已匹配软件源策略</span></div></div>
      <WhatsNew version={offer.candidateVersion ?? null} seconds={offer.versionUpdatedAtUnixSeconds} notes={offer.releaseNotes} url={offer.releaseNotesUrl}/>
      {downloadResult?.verified && <div className="download-success"><span>✓</span><div><strong>安装包校验通过</strong><p>大小、SHA-256、包名、版本和架构均通过；SHA-256：{downloadResult.actualSha256.slice(0, 16)}…</p></div></div>}
      {downloadError && <div className="inline-error">{downloadError}</div>}
      {error && <div className="inline-error">{error}</div>}

      <details className="tech-disclosure" open={downloading || ready || installRunning || installed !== null}>
        <summary><Icon name="shield"/>安全验证与安装详情<span className="tech-disclosure-hint">版本 · 来源 · 证据 · SHA-256 · 安装计划</span></summary>
        <section className="detail-section"><h3>安装版本</h3><div className="version-pair"><div><span>当前状态</span><strong>未安装</strong></div><div><span>候选版本</span><strong>{offer.candidateVersion ?? "未解析"}</strong></div></div></section>
        <InfoPanel homepage={offer.homepage} entries={[
          { label: "开发者", value: offer.vendor },
          { label: "软件包", value: offer.packageName, mono: true },
          { label: "架构", value: offer.architecture, mono: true },
          { label: "下载大小", value: downloadPlan ? formatBytes(downloadPlan.expectedSize) : "—" },
          { label: "SHA-256", value: downloadPlan?.expectedSha256 ?? "—", mono: true },
        ]}/>
        <section className="detail-section"><h3>安装来源</h3><div className="path-card"><div><span className={`source-dot ${offer.sourceKind}`}/><strong>{isWebsite ? "厂商官方发布通道" : `${offer.vendor} 官方 APT 仓库`}</strong></div><code title={downloadPlan?.downloadUrl}>{downloadPlan?.downloadUrl}</code><p>{isWebsite ? "下载地址与所有 HTTPS 重定向都不能离开允许域名。" : "安装包路径和所有 HTTPS 重定向都不能离开允许域名。"}</p></div></section>
        <section className="detail-section download-section"><h3>官方安装包</h3>
          {downloadPlan && <DownloadCard plan={downloadPlan}/>}
          <p className="download-safety">下载不请求 root 权限；校验失败的文件不会进入缓存。下载完成后需再确认，才会请求系统授权并执行安装。</p>
        </section>
        {downloadResult?.verified && <section className="detail-section final-plan-section"><h3>最终安装计划</h3>
          <div className="final-plan-card"><dl><div><dt>动作</dt><dd>{isWebsite ? "install-verified-website-deb" : "install-verified-deb"}</dd></div><div><dt>包</dt><dd>{offer.packageName} · {offer.architecture}</dd></div><div><dt>版本</dt><dd>未安装 → {downloadResult.version}</dd></div><div><dt>SHA-256</dt><dd title={downloadResult.actualSha256}>{downloadResult.actualSha256}</dd></div></dl></div>
          {operationPlan && <div className="immutable-plan"><strong>安装计划已锁定</strong><span>ID：{operationPlan.plan.planId}</span><span>installedVersion：{operationPlan.plan.payload.installedVersion ?? "null（未安装）"}</span><span>有效至：{new Date(operationPlan.plan.payload.expiresAtUnixSeconds * 1000).toLocaleTimeString("zh-CN")}</span></div>}
          {operationPlan && <DependencyGapWarning missing={operationPlan.missingDependencies}/>}
          {dryRun && <div className="dry-run-success"><strong>✓ 特权环境复核通过</strong><span>helper 已确认软件仍未安装，并重新核对来源、缓存文件与安装包元数据。</span></div>}
          <OperationLogPanel events={progressEvents} running={installPhase === "installing"}/>
          {installed && <div className="dry-run-success"><strong>✓ {offer.displayName} 安装完成</strong><span>固定参数 dpkg 安装命令已成功结束，软件列表已重新扫描。</span></div>}
        </section>}
      </details>
    </div>
  </DetailShell>;
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
  useEffect(() => {
    setState(null);
    getDevToolchainState(toolchain.toolchainId).then(setState).catch(() => { /* 状态读取失败保持待安装态 */ });
  }, [toolchain.toolchainId]);
  const installedCount = state?.installedVersions.length ?? 0;
  const statusClass = state && !state.managerFound ? "unknown" : state && installedCount > 0 ? "upToDate" : "updateAvailable";
  const statusText = state && !state.managerFound ? `未检测到 ${state.manager}` : state && installedCount > 0 ? `已安装 ${installedCount} 个版本` : "可安装";
  return <article className="app-card supported" role="button" tabIndex={0} onClick={onOpen} onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") onOpen(); }}>
    <div className="app-card-top">
      <DevLogo toolchain={toolchain}/>
      <div className="app-card-actions">
        {state && installedCount > 0
          ? <button className="ghost-link" onClick={(event) => { event.stopPropagation(); onOpen(); }} onKeyDown={(event) => event.stopPropagation()}>管理</button>
          : <button className="get-button" onClick={(event) => { event.stopPropagation(); onOpen(); }} onKeyDown={(event) => event.stopPropagation()}>获取</button>}
      </div>
    </div>
    <div className="app-card-body">
      <h3 className="app-card-name">{toolchain.displayName}</h3>
      <span className="app-card-sub">{toolchain.vendor} · {toolchain.manager}</span>
      {toolchain.description && <p className="app-card-desc">{toolchain.description}</p>}
    </div>
    <div className="app-card-footer">
      <span className={`status-badge ${statusClass}`}>{statusText}</span>
      {state?.defaultVersion && <span className="app-card-version">{state.defaultVersion}</span>}
    </div>
  </article>;
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

  return <DetailShell
    label={`${toolchain.displayName} 开发环境详情`}
    icon={<DevLogo toolchain={toolchain}/>}
    title={toolchain.displayName}
    subtitle={`${toolchain.vendor} · ${toolchain.manager}`}
    description={toolchain.description}
    canClose={busy === null}
    onClose={onClose}
  >
    <div className="drawer-content">
      {state && !state.managerFound && <div className="message"><strong>未检测到 {state.manager}</strong><span>版本管理器脚本缺失：{toolchain.managerHome}。请先安装 {state.manager}。</span></div>}
      {state?.managerFound && <div className="dev-facts">
        <div><dt>管理器</dt><dd>{state.manager} {state.managerVersion}</dd></div>
        <div><dt>默认版本</dt><dd>{state.defaultVersion ?? "未设置"}</dd></div>
        <div className="wide"><dt>管理器目录</dt><dd title={state.managerHome ?? undefined}>{state.managerHome}</dd></div>
        <div className="wide"><dt>开发者网站</dt><dd><a className="info-homepage-link" href={toolchain.homepage} onClick={(event) => { event.preventDefault(); void openExternalUrl(toolchain.homepage); }}>{toolchain.homepage}<Icon name="external"/></a></dd></div>
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
  </DetailShell>;
}

const devToolInstallKindText = { npmGlobal: "npm 全局安装", officialInstaller: "官方安装器", onPath: "本机 PATH 中的可执行文件" } as const;

function DevToolRow({ tool, state, category, onOpen }: { tool: DevTool; state: DevToolState | null; category: string; onOpen: () => void }) {
  const statusClass = state?.updateAvailable ? "updateAvailable" : state?.installed ? "upToDate" : "updateAvailable";
  const statusText = state?.updateAvailable ? "可更新" : state?.installed ? "已安装" : "可安装";
  return <article className="app-card supported" role="button" tabIndex={0} onClick={onOpen} onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") onOpen(); }}>
    <div className="app-card-top">
      <DevToolLogo tool={tool}/>
      <div className="app-card-actions">
        {!state?.installed && <button className="get-button" onClick={(event) => { event.stopPropagation(); onOpen(); }} onKeyDown={(event) => event.stopPropagation()} aria-label={`获取 ${tool.displayName}`}>获取</button>}
        {state?.updateAvailable && <button className="get-button update" onClick={(event) => { event.stopPropagation(); onOpen(); }} onKeyDown={(event) => event.stopPropagation()} aria-label={`更新 ${tool.displayName}`}>更新</button>}
        {state?.installed && state?.canUninstall && <button className="ghost-link" onClick={(event) => { event.stopPropagation(); onOpen(); }} onKeyDown={(event) => event.stopPropagation()} aria-label={`卸载 ${tool.displayName}`}>卸载</button>}
      </div>
    </div>
    <div className="app-card-body">
      <h3 className="app-card-name">{tool.displayName}</h3>
      <span className="app-card-sub">{category}</span>
      {tool.description && <p className="app-card-desc">{tool.description}</p>}
    </div>
    <div className="app-card-footer">
      <span className={`status-badge ${statusClass}`}>{statusText}</span>
      {state?.version && <span className="app-card-version">v{state.version}</span>}
    </div>
  </article>;
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
  const [busy, setBusy] = useState<"install" | "update" | "uninstall" | null>(null);
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

  const installAction = () => void run("install", (onProgress) => installDevTool(tool.toolId, onProgress));
  const updateAction = () => void run("update", (onProgress) => updateDevTool(tool.toolId, onProgress));
  const uninstallAction = () => void run("uninstall", (onProgress) => uninstallDevTool(tool.toolId, onProgress));
  const installed = state?.installed ?? false;
  const updateAvailable = state?.updateAvailable ?? false;
  const canUninstall = state?.canUninstall ?? false;
  const canInstall = tool.installer.kind === "curlScript" || (state?.npmAvailable ?? false);

  let primaryLabel: string;
  let primaryDisabled: boolean;
  let primaryOnClick: (() => void) | undefined;
  if (busy) { primaryLabel = busy === "install" ? "安装中…" : busy === "update" ? "更新中…" : "卸载中…"; primaryDisabled = true; }
  else if (!installed) { primaryLabel = "获取"; primaryDisabled = !canInstall; primaryOnClick = installAction; }
  else if (updateAvailable) { primaryLabel = "更新"; primaryDisabled = false; primaryOnClick = updateAction; }
  else { primaryLabel = "已安装"; primaryDisabled = true; }
  const heroAction = <>
    <button className="hero-button" disabled={primaryDisabled} onClick={primaryOnClick}>{primaryLabel}</button>
    {installed && canUninstall && !busy && <button className="ghost-link" onClick={uninstallAction}>卸载</button>}
  </>;

  return <DetailShell
    label={`${tool.displayName} 详情`}
    icon={<DevToolLogo tool={tool}/>}
    title={tool.displayName}
    subtitle={`${tool.vendor} · ${tool.binaryName}`}
    description={tool.description}
    action={heroAction}
    canClose={busy === null}
    onClose={onClose}
  >
    <div className="drawer-content">
      {state && !state.npmAvailable && <div className="message"><strong>未检测到 npm</strong><span>无法读取 npm 最新版本{tool.installer.kind === "npm" ? "，也无法安装该工具" : ""}。请先在“开发环境”安装并设置 Node.js。</span></div>}
      <InfoPanel homepage={tool.homepage} entries={[
        { label: "当前版本", value: state?.version ?? "未安装", mono: true },
        { label: "最新版本", value: state?.latestVersion ?? (state?.npmAvailable === false ? "无法读取" : "读取中…"), mono: true },
        { label: "安装方式", value: state?.installKind ? devToolInstallKindText[state.installKind] : "—" },
        { label: "可执行文件", value: state?.binaryPath ?? (tool.installer.kind === "npm" ? `npm 包 ${tool.npmPackage ?? ""}` : "官方安装脚本"), mono: true },
      ]}/>
      {error && <div className="inline-error">{error}</div>}
      {!installed && !canInstall && <p className="dev-empty">需要 npm 才能安装，请先在“开发环境”安装并设置 Node.js。</p>}
      <ReleaseNotes notes={state?.releaseNotes} url={state?.releaseNotesUrl}/>
      <DevToolLogPanel events={events} running={busy !== null}/>
    </div>
  </DetailShell>;
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
    <section className="software-panel store-panel">
      {error && <div className="message error"><strong>无法读取开发环境</strong><span>{error}</span></div>}
      {loading && !toolchains && <div className="empty-state"><span className="loader"/><p>正在读取运行时状态…</p></div>}
      {toolchains && <div className="app-grid">
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

function ClipboardPanel() {
  const [entries, setEntries] = useState<ClipboardEntry[]>([]);
  const [query, setQuery] = useState("");
  const [copiedId, setCopiedId] = useState<number | null>(null);

  useEffect(() => {
    let active = true;
    let lastRevision = -1;
    const load = () =>
      listClipboardHistory()
        .then((list) => { if (active) setEntries(list); })
        .catch(() => {});
    const refresh = async () => {
      try {
        const revision = await getClipboardHistoryRevision();
        if (!active || revision === lastRevision) return;
        lastRevision = revision;
        await load();
      } catch { /* 面板保持不闪断 */ }
    };
    void refresh();
    let unlisten: (() => void) | null = null;
    onClipboardHistoryChanged((list) => { if (active) setEntries(list); }).then((fn) => { unlisten = fn; });
    // 面板常驻隐藏，每次重新显示/获得焦点时重新拉取最新历史，避免展示旧列表。
    let unlistenFocus: (() => void) | null = null;
    getCurrentWebviewWindow()
      .onFocusChanged(({ payload }) => { if (payload) void refresh(); })
      .then((fn) => { unlistenFocus = fn; })
      .catch(() => {});
    const onKey = (event: KeyboardEvent) => { if (event.key === "Escape") void hideClipboardPanel(); };
    window.addEventListener("keydown", onKey);
    // 面板常驻隐藏：隐藏期间 WebKitGTK 可能挂起整个页面，导致事件/焦点回调都不可靠。
    // 这里用 visibilitychange 立即补拉 + 1s 轻量版本号轮询兜底，保证面板恢复可见时
    // 能及时刷新，且不会频繁传输带缩略图的完整列表。
    const onVisibility = () => { if (document.visibilityState === "visible") void refresh(); };
    document.addEventListener("visibilitychange", onVisibility);
    const interval = window.setInterval(() => { void refresh(); }, 1000);
    return () => {
      active = false; unlisten?.(); unlistenFocus?.();
      document.removeEventListener("visibilitychange", onVisibility);
      window.clearInterval(interval);
      window.removeEventListener("keydown", onKey);
    };
  }, []);

  const ordered = useMemo(() => [...entries.filter((entry) => entry.pinned), ...entries.filter((entry) => !entry.pinned)], [entries]);
  const visible = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return ordered;
    return ordered.filter((entry) => {
      const haystack = entry.kind === "image" ? `图片 ${entry.imageWidth ?? 0}x${entry.imageHeight ?? 0}` : (entry.text ?? "");
      return haystack.toLowerCase().includes(needle);
    });
  }, [ordered, query]);

  const copy = async (entry: ClipboardEntry) => {
    try {
      await copyClipboardEntry(entry.id);
      setCopiedId(entry.id);
      window.setTimeout(() => setCopiedId((current) => (current === entry.id ? null : current)), 1000);
    } catch { /* 面板保持不闪断 */ }
  };

  return <div className="clip-panel">
    <div className="clip-panel-top">
      <strong>剪贴板</strong>
      <label className="search-box clip-panel-search"><Icon name="search"/><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索" autoFocus/></label>
    </div>
    <div className="clip-panel-list">
      {visible.length === 0 && <div className="clip-panel-empty">{entries.length === 0 ? "还没有记录" : "无匹配"}</div>}
      {visible.map((entry) => entry.kind === "image"
        ? <div className={`clip-panel-item image ${copiedId === entry.id ? "copied" : ""}`} key={entry.id} draggable
            onClick={() => void copy(entry)} title="点击复制图片，或拖到聊天窗口发送文件"
            onDragStart={(event) => { event.preventDefault(); dragClipboardImage(entry.id).catch((reason) => console.error("图片拖拽失败", reason)); }}>
            <img className="clip-panel-thumb" draggable={false} src={entry.imagePreview ?? ""} alt="剪贴板图片"/>
            <span className="clip-panel-dim">{entry.imageWidth ?? 0}×{entry.imageHeight ?? 0}</span>
          </div>
        : <div className={`clip-panel-item text ${copiedId === entry.id ? "copied" : ""}`} key={entry.id} onClick={() => void copy(entry)} title="点击复制">
            <span className="clip-panel-text">{entry.text}</span>
          </div>)}
    </div>
  </div>;
}

function ClipboardPage() {
  const [entries, setEntries] = useState<ClipboardEntry[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [copiedId, setCopiedId] = useState<number | null>(null);
  const [pendingId, setPendingId] = useState<number | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const [previewEntry, setPreviewEntry] = useState<ClipboardEntry | null>(null);
  const [hotkey, setHotkey] = useState<string | null>(null);
  const [hotkeyDraft, setHotkeyDraft] = useState("");
  const [hotkeySaving, setHotkeySaving] = useState(false);
  const [hotkeySaved, setHotkeySaved] = useState(false);
  const [session, setSession] = useState<SessionInfo | null>(null);

  useEffect(() => {
    let active = true;
    listClipboardHistory()
      .then((list) => { if (active) setEntries(list); })
      .catch((reason) => { if (active) setError(String(reason)); })
      .finally(() => { if (active) setLoading(false); });
    let unlisten: (() => void) | null = null;
    onClipboardHistoryChanged((list) => setEntries(list)).then((fn) => { unlisten = fn; });
    getClipboardHotkey().then((value) => { if (active) { setHotkey(value); setHotkeyDraft(value); } }).catch(() => {});
    getSessionInfo().then((info) => { if (active) setSession(info); }).catch(() => {});
    return () => { active = false; unlisten?.(); };
  }, []);

  // 主窗口关闭到托盘再恢复时，也重新拉取最新剪贴板历史。
  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    let active = true;
    let unlistenFocus: (() => void) | null = null;
    const reload = () =>
      listClipboardHistory()
        .then((list) => { if (active) { setEntries(list); setLoading(false); } })
        .catch((reason) => { if (active) setError(String(reason)); });
    getCurrentWebviewWindow()
      .onFocusChanged(({ payload }) => { if (payload) reload(); })
      .then((fn) => { unlistenFocus = fn; })
      .catch(() => {});
    return () => { active = false; unlistenFocus?.(); };
  }, []);

  const copy = async (entry: ClipboardEntry) => {
    setError(null);
    try {
      await copyClipboardEntry(entry.id);
      setCopiedId(entry.id);
      window.setTimeout(() => setCopiedId((current) => (current === entry.id ? null : current)), 1300);
    } catch (reason) {
      setError(String(reason));
    }
  };

  const togglePin = async (entry: ClipboardEntry) => {
    setError(null); setPendingId(entry.id);
    try { await setClipboardEntryPinned(entry.id, !entry.pinned); } catch (reason) { setError(String(reason)); }
    finally { setPendingId(null); }
  };

  const remove = async (entry: ClipboardEntry) => {
    setError(null); setPendingId(entry.id);
    try { await deleteClipboardEntry(entry.id); } catch (reason) { setError(String(reason)); }
    finally { setPendingId(null); }
  };

  const dragOut = (entry: ClipboardEntry) => {
    setError(null);
    dragClipboardImage(entry.id).catch((reason) => setError(String(reason)));
  };

  const saveHotkey = async () => {
    const value = hotkeyDraft.trim();
    if (!value || value === hotkey) return;
    setHotkeySaving(true); setHotkeySaved(false); setError(null);
    try {
      const saved = await setClipboardHotkey(value);
      setHotkey(saved); setHotkeyDraft(saved); setHotkeySaved(true);
      window.setTimeout(() => setHotkeySaved(false), 1500);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setHotkeySaving(false);
    }
  };

  const clearAll = async () => {
    if (!confirmClear) {
      setConfirmClear(true);
      window.setTimeout(() => setConfirmClear(false), 3000);
      return;
    }
    setError(null);
    try { await clearClipboardHistory(); } catch (reason) { setError(String(reason)); }
    finally { setConfirmClear(false); }
  };

  const ordered = useMemo(() => {
    const list = entries ?? [];
    return [...list.filter((entry) => entry.pinned), ...list.filter((entry) => !entry.pinned)];
  }, [entries]);
  const visible = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return ordered;
    return ordered.filter((entry) => {
      const haystack = entry.kind === "image"
        ? `图片 ${entry.imageWidth ?? 0}x${entry.imageHeight ?? 0}`
        : (entry.text ?? "");
      return haystack.toLowerCase().includes(needle);
    });
  }, [ordered, query]);

  return <main className="workspace clipboard-workspace">
    <header className="workspace-header"><div><h1>剪贴板</h1><p>运行期间自动记录 · 关闭窗口收起到托盘，Alt+Shift+V 随时唤出</p></div><div className="header-actions"><span className="clipboard-count">{entries ? `${entries.length} 条` : ""}</span><button className="primary-button danger" onClick={() => void clearAll()} disabled={!entries || entries.length === 0}>{confirmClear ? "再点一次确认清空" : "清空历史"}</button></div></header>
    <section className="software-panel clipboard-panel">
      <div className="panel-toolbar">
        <div className="filter-tabs">
          <span className="clipboard-hint">本机轮询读取、内容不上传，最多 500 条；关闭窗口会收起到托盘，全局热键 Alt+Shift+V 唤出（Wayland 上热键可能不可用）。</span>
        </div>
        <label className="search-box"><Icon name="search"/><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索剪贴板内容"/></label>
      </div>
      <div className="clipboard-hotkey-row">
        <span>全局热键唤出面板</span>
        {session && <span className={`clip-session-badge ${session.kind}`}>{session.globalHotkeySupported ? "X11 可用" : session.kind === "wayland" ? "Wayland 受限" : "未识别会话"}</span>}
        <input className="clipboard-hotkey-input" value={hotkeyDraft} onChange={(event) => setHotkeyDraft(event.target.value)} disabled={hotkeySaving} spellCheck={false}/>
        <button className="dev-action-button subtle" onClick={() => void saveHotkey()} disabled={hotkeySaving || !hotkeyDraft.trim() || hotkeyDraft.trim() === hotkey}>{hotkeySaved ? "已保存 ✓" : "保存热键"}</button>
      </div>
      {session && session.kind !== "x11" && <div className={`clipboard-session-note ${session.kind}`}>{session.kind === "wayland"
        ? <>检测到 <b>Wayland</b> 会话：应用内全局热键不生效，请到<b>系统设置 → 键盘 → 查看及自定义快捷键 → 自定义快捷键</b>绑定 <b>Super+V</b> → 命令 <code>umanager --toggle-clipboard-panel</code>（由 GNOME 调用本应用）。快捷面板已切换为 XWayland 后端，会定位在右上角托盘旁。</>
        : <>未识别到 X11/Wayland 会话，全局热键可能不可用；建议用系统自定义快捷键绑定 <code>umanager --toggle-clipboard-panel</code>。</>}</div>}
      {error && <div className="message error"><strong>剪贴板操作失败</strong><span>{error}</span></div>}
      {loading && !entries && <div className="empty-state"><span className="loader"/><p>正在读取剪贴板历史…</p></div>}
      {entries && entries.length === 0 && <div className="empty-state"><p>还没有记录。复制文本或截图后会自动出现在这里。</p></div>}
      {entries && entries.length > 0 && visible.length === 0 && <div className="empty-state"><p>没有匹配的记录</p></div>}
      <div className="clipboard-list">
        {visible.map((entry) => <div className={`clip-entry ${entry.pinned ? "pinned" : ""}`} key={entry.id}>
          <div className="clip-entry-meta">
            <span className="clip-time">{new Date(entry.capturedAtMs).toLocaleString("zh-CN", { hour12: false })}</span>
            <span className="clip-chars">{entry.kind === "image" ? `${entry.imageWidth ?? 0}×${entry.imageHeight ?? 0} · ${formatBytes(entry.imageByteCount ?? 0)}` : `${entry.charCount ?? 0} 字符`}</span>
            {entry.kind === "image" && <span className="clip-drag-hint">拖动=发送文件</span>}
            {entry.pinned && <span className="status-badge upToDate">已置顶</span>}
          </div>
          {entry.kind === "image"
            ? <button className="clip-image-thumb" draggable onDragStart={(event) => { event.preventDefault(); dragOut(entry); }} onClick={() => setPreviewEntry(entry)} title="拖动到聊天窗口或文件管理器即可发送/保存；点击查看大图"><img draggable={false} src={entry.imagePreview ?? ""} alt="剪贴板图片"/></button>
            : <pre className="clip-text">{entry.text}</pre>}
          <div className="clip-actions">
            <button className="dev-action-button" disabled={pendingId === entry.id} onClick={() => void copy(entry)}>{copiedId === entry.id ? "已复制 ✓" : entry.kind === "image" ? "复制图片" : "复制"}</button>
            <button className="dev-action-button subtle" disabled={pendingId === entry.id} onClick={() => void togglePin(entry)}>{entry.pinned ? "取消置顶" : "置顶"}</button>
            <button className="dev-action-button danger-ghost" disabled={pendingId === entry.id} onClick={() => void remove(entry)}>删除</button>
          </div>
        </div>)}
      </div>
    </section>
    {previewEntry && <ClipboardImageDialog entry={previewEntry} onClose={() => setPreviewEntry(null)} onCopied={() => setCopiedId(previewEntry.id)}/>}
  </main>;
}

function ClipboardImageDialog({ entry, onClose, onCopied }: { entry: ClipboardEntry; onClose: () => void; onCopied: () => void }) {
  const [src, setSrc] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let active = true;
    setSrc(null); setError(null);
    getClipboardImage(entry.id)
      .then((dataUrl) => { if (active) setSrc(dataUrl); })
      .catch((reason) => { if (active) setError(String(reason)); });
    return () => { active = false; };
  }, [entry.id]);

  const copyFull = async () => {
    setError(null);
    try {
      await copyClipboardEntry(entry.id);
      setCopied(true); onCopied();
      window.setTimeout(() => setCopied(false), 1300);
    } catch (reason) {
      setError(String(reason));
    }
  };

  const focusRef = useModalFocus<HTMLElement>(onClose, true);
  return <section className="clip-image-overlay" role="dialog" aria-modal="true" aria-label="剪贴板图片" ref={focusRef} tabIndex={-1}>
    <div className="clip-image-dialog">
      <header className="clip-image-head">
        <div><strong>剪贴板图片</strong><span>{entry.imageWidth ?? 0}×{entry.imageHeight ?? 0} · {formatBytes(entry.imageByteCount ?? 0)}</span></div>
        <button className="dev-action-button danger-ghost" onClick={onClose}>关闭</button>
      </header>
      {error && <div className="message error"><strong>无法加载图片</strong><span>{error}</span></div>}
      <div className="clip-image-body">{src ? <img src={src} alt="剪贴板图片"/> : <span className="loader"/>}</div>
      <div className="clip-actions"><button className="dev-action-button" onClick={() => void copyFull()}>{copied ? "已复制 ✓" : "复制图片"}</button></div>
    </div>
  </section>;
}

export default function App() {
  if (clipboardPanelMode()) return <ClipboardPanel/>;
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
  const [pendingLocalDeb, setPendingLocalDeb] = useState<LocalDebInspection | null>(null);
  const [pendingLocalDebError, setPendingLocalDebError] = useState<string | null>(null);
  const [catalog, setCatalog] = useState<CatalogApplication[] | null>(null);
  const [categoryCatalog, setCategoryCatalog] = useState<CategoryCatalog | null>(null);
  const [devTools, setDevTools] = useState<DevTool[] | null>(null);
  const [devToolStates, setDevToolStates] = useState<Record<string, DevToolState | null>>({});
  const [devToolsError, setDevToolsError] = useState<string | null>(null);
  const [selectedDevTool, setSelectedDevTool] = useState<DevTool | null>(null);
  const [downloads, setDownloads] = useState<Record<string, DownloadState>>({});
  const [notice, setNotice] = useState<string | null>(null);

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
  const updatableItems = useMemo(() => softwareItems.filter((item) => item.kind === "deb" ? item.deb!.updateState === "updateAvailable" : item.toolState?.updateAvailable === true), [softwareItems]);
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
  const applicationIdOf = (packageName: string) => catalogByPackage[packageName]?.applicationId;
  const downloadProgressOf = (packageName: string): DownloadProgress | null => {
    const state = downloads[packageName];
    return state && (state.status === "downloading" || state.status === "verifying") ? state.progress : null;
  };
  const startDownload = async (applicationId: string, packageName: string, notify: { title: string; body: string }) => {
    const current = downloads[packageName];
    if (current && (current.status === "downloading" || current.status === "verifying" || current.status === "ready")) return;
    setDownloads((prev) => ({ ...prev, [packageName]: { status: "downloading", progress: { packageName, phase: "downloading", transferredBytes: 0, totalBytes: 0, bytesPerSecond: 0 } } }));
    try {
      const result = await downloadPackage(applicationId, packageName, (progress) => {
        setDownloads((prev) => ({ ...prev, [packageName]: { status: progress.phase === "verifying" ? "verifying" : "downloading", progress } }));
      });
      if (!result.verified) {
        setDownloads((prev) => ({ ...prev, [packageName]: { status: "error", error: "安装包校验未通过，已停止，未更改系统。" } }));
        return;
      }
      setDownloads((prev) => ({ ...prev, [packageName]: { status: "ready", result } }));
      if (!document.hasFocus()) {
        void notifyDownloadComplete(notify.title, notify.body).catch(() => { /* 通知失败不影响流程 */ });
      }
    } catch (reason) {
      setDownloads((prev) => ({ ...prev, [packageName]: { status: "error", error: String(reason) } }));
    }
  };
  const clearDownload = (packageName: string) => {
    setDownloads((prev) => { const next = { ...prev }; delete next[packageName]; return next; });
  };
  const launchApp = (packageName: string) => {
    const applicationId = catalogByPackage[packageName]?.applicationId;
    if (!applicationId) { setNotice(`软件源中未找到 ${packageName} 的适配策略`); return; }
    launchApplication(applicationId).catch((reason) => setNotice(String(reason)));
  };
  const showUpdatesPage = () => {
    setPage("updates");
    setUpdatePackage(null); setRemovalPackage(null); setInstallOffer(null); setSelectedDevTool(null);
    if (!installableOffers && !installableLoading) void refreshInstallable();
  };
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
  const showClipboardPage = () => {
    setPage("clipboard");
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
        <div className="nav-section">商店</div>
        <button className={`nav-item ${page === "installed" ? "active" : ""}`} onClick={showInstalledPage}><Icon name="apps"/>软件</button>
        <button className={`nav-item ${page === "updates" ? "active" : ""}`} onClick={showUpdatesPage}><Icon name="update"/>更新{updatesCount > 0 && <span className="nav-badge">{updatesCount}</span>}</button>
        <div className="nav-section">工具</div>
        <button className={`nav-item ${page === "scripts" ? "active" : ""}`} onClick={showScriptsPage}><Icon name="script"/>维护脚本</button>
        <button className={`nav-item ${page === "dev" ? "active" : ""}`} onClick={showDevToolsPage}><Icon name="dev"/>开发环境</button>
        <button className={`nav-item ${page === "clipboard" ? "active" : ""}`} onClick={showClipboardPage}><Icon name="clipboard"/>剪贴板</button>
      </nav>
      <div className="sidebar-spacer"/>
      <div className="safety-card"><Icon name="shield"/><div><strong>安全更新</strong><span>仅在确认授权后更改系统</span></div></div>
      <button className={`nav-item settings ${page === "settings" ? "active" : ""}`} onClick={showSettingsPage}><Icon name="settings"/>设置</button>
      <div className="version-label">UManager {installationInfo?.appVersion ?? appVersion ?? ""}</div>
    </aside>

    {page === "installed" ? <main className="workspace">
      <header className="workspace-header">
        <div><h1>软件</h1><p>桌面应用、命令行工具与 AI 工具</p></div>
        <div className="header-actions">
          {result && <time>上次检查 {new Date(result.scannedAtUnixSeconds * 1000).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })}</time>}
          <label className="store-search"><Icon name="search"/><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索软件"/></label>
          <button className="primary-button" onClick={refreshAll} disabled={scanning}><span className={scanning ? "spin" : ""}>↻</span>{scanning ? "检查中…" : "检查更新"}</button>
        </div>
      </header>
      <section className="software-panel store-panel">
        {pendingLocalDebError && <div className="message error"><strong>无法打开本地 .deb</strong><span>{pendingLocalDebError}</span></div>}
        <div className="category-bar">
          <button className={`category-chip ${categoryFilter === "全部" ? "active" : ""}`} onClick={() => setCategoryFilter("全部")}>全部</button>
          {categoryChips.map((category) => <button key={category} className={`category-chip ${categoryFilter === category ? "active" : ""}`} onClick={() => setCategoryFilter(category)}>{category}</button>)}
        </div>
        <div className="panel-toolbar">
          <div className="filter-tabs segmented">
            <button className={filter === "all" ? "active" : ""} onClick={() => setFilter("all")}>全部</button>
            <button className={filter === "installed" ? "active" : ""} onClick={() => setFilter("installed")}>已安装</button>
            <button className={filter === "updates" ? "active" : ""} onClick={() => setFilter("updates")}>可更新 {updatesCount > 0 && <b>{updatesCount}</b>}</button>
            <button className={filter === "installable" ? "active" : ""} onClick={() => setFilter("installable")}>可安装</button>
          </div>
        </div>
        {error && <div className="message error"><strong>无法读取软件信息</strong><span>{error}</span></div>}
        {installableError && <div className="message error"><strong>无法读取可安装软件</strong><span>{installableError}</span></div>}
        {devToolsError && <div className="message error"><strong>无法读取 CLI 工具</strong><span>{devToolsError}</span></div>}
        {result?.warnings.map((warning) => <div className="message" key={warning}>{warning}</div>)}
        {!result && !installableOffers && !devTools && <div className="empty-state"><span className="loader"/><p>正在读取已安装与可安装软件…</p></div>}
        {result && visibleSoftware.length === 0 && <div className="empty-state"><p>没有符合条件的软件</p></div>}
        <div className="app-grid">{visibleSoftware.map((item) => item.kind === "deb"
          ? <SoftwareRow item={item.deb!} category={item.category} progress={downloadProgressOf(item.deb!.packageName)} onOpen={() => openSoftware(item)} onRemove={() => { if (item.deb!.managed) setRemovalPackage(item.deb!.managed); }} onLaunch={() => launchApp(item.deb!.packageName)} key={item.key}/>
          : <DevToolRow tool={item.tool!} state={item.toolState ?? null} category={item.category} onOpen={() => openSoftware(item)} key={item.key}/>)}</div>
      </section>
    </main> : page === "updates" ? <main className="workspace store-workspace">
      <header className="workspace-header">
        <div><h1>更新</h1><p>可用更新的软件与命令行工具</p></div>
        <div className="header-actions">
          <button className="secondary-button" onClick={() => setNotice("「全部更新」暂未实现，请逐项更新。")}>全部更新</button>
          <button className="primary-button" onClick={refreshAll} disabled={scanning}><span className={scanning ? "spin" : ""}>↻</span>{scanning ? "检查中…" : "检查更新"}</button>
        </div>
      </header>
      <section className="software-panel store-panel">
        {error && <div className="message error"><strong>无法读取软件信息</strong><span>{error}</span></div>}
        {installableError && <div className="message error"><strong>无法读取可安装软件</strong><span>{installableError}</span></div>}
        {devToolsError && <div className="message error"><strong>无法读取 CLI 工具</strong><span>{devToolsError}</span></div>}
        {!result && !installableOffers && !devTools && <div className="empty-state"><span className="loader"/><p>正在读取更新状态…</p></div>}
        {result && updatableItems.length === 0 && <div className="empty-state"><p>所有软件均为最新版本。</p></div>}
        <div className="app-grid">{updatableItems.map((item) => item.kind === "deb"
          ? <SoftwareRow item={item.deb!} category={item.category} progress={downloadProgressOf(item.deb!.packageName)} onOpen={() => openSoftware(item)} onRemove={() => { if (item.deb!.managed) setRemovalPackage(item.deb!.managed); }} onLaunch={() => launchApp(item.deb!.packageName)} key={item.key}/>
          : <DevToolRow tool={item.tool!} state={item.toolState ?? null} category={item.category} onOpen={() => openSoftware(item)} key={item.key}/>)}</div>
      </section>
    </main> : page === "dev" ? <DevToolsPage/> : page === "scripts" ? <ScriptsPage/> : page === "clipboard" ? <ClipboardPage/> : <SettingsPage info={installationInfo} loading={installationInfoLoading} error={installationInfoError} onRefresh={() => void refreshInstallationInfo()}/>}
    {updatePackage && <UpdateDrawer item={updatePackage} download={downloads[updatePackage.packageName]} onStartDownload={(notify) => void startDownload(applicationIdOf(updatePackage.packageName) ?? "", updatePackage.packageName, notify)} onClearDownload={() => clearDownload(updatePackage.packageName)} onClose={() => setUpdatePackage(null)} onInstalled={() => void refresh()} onLaunch={() => launchApp(updatePackage.packageName)} onRemove={() => { setUpdatePackage(null); setRemovalPackage(updatePackage); }}/>}
    {pendingLocalDeb && <LocalDebDialog initial={pendingLocalDeb} onClose={() => setPendingLocalDeb(null)} onInstalled={() => void refresh()}/>}
    {removalPackage && <RemovalDialog item={removalPackage} onClose={() => setRemovalPackage(null)} onRemoved={() => void refresh()}/>}
    {installOffer && (
      <InstallDrawer offer={installOffer} download={downloads[installOffer.packageName]} onStartDownload={(notify) => void startDownload(installOffer.applicationId, installOffer.packageName, notify)} onClearDownload={() => clearDownload(installOffer.packageName)} onClose={() => setInstallOffer(null)} onInstalled={() => { void refresh(); void refreshInstallable(); }} onLaunch={() => launchApp(installOffer.packageName)}/>
    )}
    {selectedDevTool && (
      <DevToolDrawer tool={selectedDevTool} onClose={() => setSelectedDevTool(null)} onChanged={() => void loadDevTools()}/>
    )}
    {notice && <div className="app-notice" role="status" onClick={() => setNotice(null)}><span>{notice}</span><button aria-label="关闭">×</button></div>}
  </div>;
}
