// Windows 应用（Wine）管理的前端组件：列表卡片与操作复核对话框。企业微信
// 作为普通软件条目融入「软件 / 更新」页，详情（含 Wine 配置）在 App 层的
// 详情抽屉中展示；安装包授权与校验全部来自签名 feed。
import { useEffect, useRef } from "react";
import type { DownloadProgress, WindowsAction, WindowsPlan, WindowsSettings, WindowsState } from "./types";
import { CardDownloadRing } from "./DownloadProgress";
import wecomIcon from "./assets/app-icons/wecom.png";

export type WindowsRowAction = "install" | "update" | "uninstall" | "launch" | "configure";

const labels: Record<WindowsAction, string> = { install: "安装", update: "更新", uninstall: "卸载", configure: "应用 Wine 配置" };

// 列表卡片：结构与普通软件卡片一致（图标 / 操作 / 名称 / 分类 / 描述 / 状态），
// 不改变软件列表的布局。详情与 Wine 配置通过 onOpen 打开详情抽屉。
export function WindowsRow({ state, progress, category, onOpen, onLaunch, onRemove }: {
  state: WindowsState;
  progress: DownloadProgress | null;
  category: string;
  onOpen: () => void;
  onLaunch: () => void;
  onRemove: () => void;
}) {
  const statusText = !state.installed ? "未安装" : state.updateAvailable ? "可更新" : "已安装";
  const statusClass = !state.installed || state.updateAvailable ? "updateAvailable" : "upToDate";
  const downloading = progress !== null && (progress.phase === "downloading" || progress.phase === "verifying");
  return <article className="app-card supported" role="button" tabIndex={0} onClick={onOpen} onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); onOpen(); } }}>
    <div className="app-card-top">
      <span className="app-mark has-icon"><img src={wecomIcon} alt=""/></span>
      <div className="app-card-actions">
        {downloading && progress
          ? <CardDownloadRing progress={progress}/>
          : !state.installed
            ? <button className="get-button" onClick={(event) => { event.stopPropagation(); onOpen(); }} onKeyDown={(event) => event.stopPropagation()} aria-label={`获取 ${"企业微信"}`}>获取</button>
            : <>
              {state.updateAvailable && <button className="get-button update" onClick={(event) => { event.stopPropagation(); onOpen(); }} onKeyDown={(event) => event.stopPropagation()} aria-label={`更新 ${"企业微信"}`}>更新</button>}
              <button className="get-button open" onClick={(event) => { event.stopPropagation(); onLaunch(); }} onKeyDown={(event) => event.stopPropagation()} aria-label={`打开 ${"企业微信"}`}>打开</button>
              <button className="ghost-link" onClick={(event) => { event.stopPropagation(); onRemove(); }} onKeyDown={(event) => event.stopPropagation()} aria-label={`卸载 ${"企业微信"}`}>卸载</button>
            </>}
      </div>
    </div>
    <div className="app-card-body">
      <h3 className="app-card-name">企业微信</h3>
      <span className="app-card-sub">{category} · Wine</span>
      <p className="app-card-desc">Windows 版企业微信，通过本机 Wine 运行。</p>
    </div>
    <div className="app-card-footer">
      <span className={`status-badge ${statusClass}`}>{statusText}</span>
      <span className="app-card-version">{state.installedVersion ?? state.candidateVersion ?? (state.installed ? "无法识别" : "未安装")}</span>
    </div>
  </article>;
}

// 操作复核对话框：安装 / 更新 / 卸载 / 配置共用，展示计划摘要并确认执行。
export function WindowsConfirmDialog({ plan, busy, message, onConfirm, onCancel }: {
  plan: WindowsPlan;
  busy: boolean;
  message: string;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => { ref.current?.showModal(); }, []);
  return <dialog ref={ref} className="windows-confirm" aria-labelledby="windows-confirm-title" onCancel={(event) => { if (busy) event.preventDefault(); else onCancel(); }}>
    <h2 id="windows-confirm-title">确认{labels[plan.action]}</h2>
    <p>企业微信 {plan.installedVersion ?? "未安装"}{plan.targetVersion ? ` → ${plan.targetVersion}` : ""}</p>
    <p className="windows-note">环境：{plan.prefix}</p>
    <p>{plan.settings.windowsVersion === "win10" ? "Windows 10" : "Windows 11"} · {plan.settings.dpi} DPI · {plan.settings.graphicsDriver}</p>
    {plan.sha256 && <><p>安装包已通过签名软件源的大小与 SHA-256 校验（{((plan.downloadSize ?? 0) / 1024 / 1024).toFixed(1)} MB）。</p><code className="windows-hash">{plan.sha256}</code></>}
    <p>{plan.action === "uninstall" ? "将打开官方卸载向导。请在向导中选择是否保留聊天记录；UManager 保留 Wine 环境目录。" : plan.action === "configure" ? "将修改企业微信环境的兼容设置，下次启动时生效。" : "将打开官方安装向导，请保持默认安装目录。完成后退出企业微信，UManager 会检查实际安装版本。"}</p>
    <p className="windows-note">操作计划有效至 {new Date(plan.expiresAt * 1000).toLocaleTimeString("zh-CN")}，以当前用户执行。</p>
    {busy && <p role="status">{message || "操作进行中，请完成官方面导…"}</p>}
    <div className="windows-actions">
      <button className="secondary-button" disabled={busy} onClick={onCancel}>取消</button>
      <button className="primary-button" disabled={busy} onClick={onConfirm}>{busy ? "执行中…" : `确认${labels[plan.action]}`}</button>
    </div>
  </dialog>;
}

export type { WindowsSettings };
