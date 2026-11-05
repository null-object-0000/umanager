import { useEffect, useState } from "react";
import { executeArchiveOperation, getDownloadPlan, launchApplication, prepareArchiveOperation } from "./api";
import antigravityIcon from "./assets/app-icons/antigravity.png";
import { useModalFocus } from "./useModalFocus";
import type { ArchivePlan, CatalogApplication, DownloadJob, DownloadPlan } from "./types";

export function ArchiveDrawer({ application, installedVersion, remove = false, job, onClose, onChanged, onEnqueue, onCancelDownload, onDismissDownload }: {
  application: CatalogApplication; installedVersion: string | null; remove?: boolean; job?: DownloadJob;
  onClose: () => void; onChanged: () => void; onEnqueue: () => void; onCancelDownload: () => void; onDismissDownload: () => void;
}) {
  const [download, setDownload] = useState<DownloadPlan | null>(null);
  const [plan, setPlan] = useState<ArchivePlan | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState(false);
  useEffect(() => {
    let active = true;
    if (!remove) void getDownloadPlan(application.applicationId).then((p) => { if (active) setDownload(p); }).catch((e) => { if (active) setError(String(e)); });
    return () => { active = false; };
  }, [application.applicationId, remove]);
  const run = async (execute: boolean) => {
    setBusy(true); setError(null);
    try {
      if (execute && plan) {
        setMessage(await executeArchiveOperation(plan.planId));
        setDone(true); onDismissDownload(); onChanged();
      } else setPlan(await prepareArchiveOperation(application.applicationId, remove));
    } catch (e) { setError(String(e)); setPlan(null); }
    finally { setBusy(false); }
  };
  const focusRef = useModalFocus<HTMLElement>(onClose, !busy);
  const active = job && ["queued", "downloading", "verifying"].includes(job.status);
  const ready = job?.status === "ready";
  const upToDate = installedVersion !== null && installedVersion === download?.version;
  const removing = plan?.payload.remove ?? remove;
  const label = removing ? "卸载" : installedVersion ? "更新" : "安装";
  return <div className="local-deb-layer">
    <section ref={focusRef} tabIndex={-1} className="local-deb-dialog" role="dialog" aria-modal="true" aria-label={`${label} ${application.displayName}`}>
      <header><div>{application.icon === "antigravity" && <span className="app-mark has-icon"><img src={antigravityIcon} alt=""/></span>}<div><h2>{application.displayName}</h2><p>{application.vendor} · 官网 tar.gz · 当前用户</p></div></div><button className="close-button" onClick={onClose} disabled={busy} aria-label="关闭">×</button></header>
      <div className="local-deb-content">
        <p>{application.description}</p>
        <p>{remove ? "移除 UManager 安装的程序、旧版本和桌面启动项，保留应用个人数据。" : "安装到当前用户目录，无需管理员授权。下载来自签名软件源，安装前检查 SHA-256、归档路径、版本和程序架构。"}</p>
        <dl className="local-deb-facts">
          <div><dt>已安装</dt><dd>{installedVersion ?? "未安装"}</dd></div>
          {!remove && <div><dt>官网版本</dt><dd>{download?.version ?? "读取中…"}</dd></div>}
          {download && <div className="wide"><dt>下载文件</dt><dd>{download.fileName} · {(download.expectedSize / 1024 / 1024).toFixed(1)} MB</dd></div>}
          {plan && <><div className="wide"><dt>安装目录</dt><dd>{plan.installPath}</dd></div><div className="wide"><dt>SHA-256</dt><dd>{plan.payload.sha256}</dd></div><div className="wide"><dt>计划</dt><dd>{plan.planId}</dd></div><div><dt>有效至</dt><dd>{new Date(plan.payload.expiresAtUnixSeconds * 1000).toLocaleTimeString("zh-CN")}</dd></div></>}
        </dl>
        {job && active && <p role="status">{job.status === "queued" ? "已加入后台下载队列" : job.status === "verifying" ? "正在校验安装包…" : `正在后台下载… ${Math.round((job.progress?.transferredBytes ?? 0) / Math.max(job.progress?.totalBytes ?? 1, 1) * 100)}%`}</p>}
        {(error || job?.status === "error") && <div className="message error" role="alert">{error ?? job?.error}</div>}
        {message && <div className="dry-run-success" role="status">{message}</div>}
        {done ? <>{!removing && <button className="download-button" onClick={() => void launchApplication(application.applicationId).catch((e) => setError(String(e)))}>打开</button>}<button className="secondary-button" onClick={onClose}>关闭</button></> : plan ? <><p>已锁定上述计划。确认后将{label} {application.displayName} {plan.payload.version}。</p><button className="download-button" disabled={busy} onClick={() => void run(true)}>{busy ? "正在执行…" : `确认${label}`}</button><button className="secondary-button" disabled={busy} onClick={() => setPlan(null)}>返回</button></> : <>
          {active ? <button className="secondary-button" onClick={onCancelDownload}>取消下载</button> : !remove && !ready && !upToDate ? <button className="download-button" disabled={!download || busy} onClick={onEnqueue}>{job?.status === "error" ? "重新下载" : "获取"}</button> : upToDate && !remove ? <button className="download-button" onClick={() => void launchApplication(application.applicationId).catch((e) => setError(String(e)))}>打开</button> : <button className="download-button" disabled={busy} onClick={() => void run(false)}>{busy ? "正在复核…" : `准备${label}计划`}</button>}
          {!remove && installedVersion && <button className="secondary-button" disabled={busy} onClick={async () => { setBusy(true); setError(null); try { setPlan(await prepareArchiveOperation(application.applicationId, true)); } catch (e) { setError(String(e)); } finally { setBusy(false); } }}>卸载</button>}
        </>}
      </div>
    </section>
  </div>;
}
