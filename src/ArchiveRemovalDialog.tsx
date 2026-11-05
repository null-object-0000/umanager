import { useState } from "react";
import { executeArchiveOperation, prepareArchiveOperation } from "./api";
import { useModalFocus } from "./useModalFocus";
import antigravityIcon from "./assets/app-icons/antigravity.png";
import type { ArchivePlan, CatalogApplication } from "./types";

export function ArchiveRemovalDialog({ application, installedVersion, onClose, onChanged }: {
  application: CatalogApplication; installedVersion: string; onClose: () => void; onChanged: () => void;
}) {
  const [confirmed, setConfirmed] = useState(false);
  const [plan, setPlan] = useState<ArchivePlan | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const focusRef = useModalFocus<HTMLElement>(onClose, !busy);
  const run = async () => {
    if (busy) return;
    setBusy(true); setError(null);
    try {
      if (!plan) setPlan(await prepareArchiveOperation(application.applicationId, true));
      else { await executeArchiveOperation(plan.planId); onChanged(); }
    } catch (e) { setError(String(e)); setPlan(null); }
    finally { setBusy(false); }
  };
  return <div className="local-deb-layer">
    <section ref={focusRef} tabIndex={-1} className="local-deb-dialog removal-dialog" role="dialog" aria-modal="true" aria-label={`卸载 ${application.displayName}`}>
      <header><div>{application.icon === "antigravity" && <span className="app-mark has-icon"><img src={antigravityIcon} alt=""/></span>}<div><h2>卸载 {application.displayName}</h2><p>{application.vendor} · {application.packageName}</p></div></div><button className="close-button" onClick={onClose} disabled={busy} aria-label="关闭">×</button></header>
      <div className="local-deb-content">
        <div className="removal-warning"><strong>这会从当前用户目录中移除该软件</strong><span>删除 UManager 安装的程序、旧版本和桌面启动项，保留应用个人数据，无需管理员权限。</span></div>
        <dl className="local-deb-facts"><div><dt>应用</dt><dd>{application.packageName}</dd></div><div><dt>已安装</dt><dd>{installedVersion}</dd></div>{plan && <div className="wide"><dt>移除目录</dt><dd>{plan.installPath}</dd></div>}</dl>
        {error && <div className="message error" role="alert">{error}</div>}
        {plan ? <><div className="immutable-plan"><strong>卸载计划已锁定</strong><span>有效至：{new Date(plan.payload.expiresAtUnixSeconds * 1000).toLocaleTimeString("zh-CN")}</span></div><button className="download-button" disabled={busy} onClick={() => void run()}>{busy ? "正在卸载…" : "确认卸载"}</button></> : <><label className="plan-confirmation removal-confirmation"><input type="checkbox" checked={confirmed} disabled={busy} onChange={(event) => setConfirmed(event.target.checked)}/><span>我已核对应用、版本和卸载范围，确认移除程序并保留个人数据。</span></label><button className="download-button" disabled={!confirmed || busy} onClick={() => void run()}>{busy ? "正在复核…" : "确认并锁定卸载计划"}</button></>}
      </div>
    </section>
  </div>;
}
