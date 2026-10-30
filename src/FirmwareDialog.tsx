import { useEffect, useRef, useState } from "react";
import { installDeviceFirmware, prepareFirmwareUpdate, runFirmwareDryRun } from "./api";
import type { FirmwarePlan, FirmwareReport, OperationProgressEvent } from "./types";

export function FirmwareDialog({ deviceId, version, onClose, onUpdated }: {
  deviceId: string; version: string; onClose: () => void; onUpdated: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [plan, setPlan] = useState<FirmwarePlan | null>(null);
  const [verified, setVerified] = useState(false);
  const [firstConfirmed, setFirstConfirmed] = useState(false);
  const [secondConfirmed, setSecondConfirmed] = useState(false);
  const [busy, setBusy] = useState<"plan" | "verify" | "install" | null>("plan");
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<FirmwareReport | null>(null);
  const [failed, setFailed] = useState(false);
  const [events, setEvents] = useState<OperationProgressEvent[]>([]);
  const [now, setNow] = useState(Date.now());
  const operation = useRef(false);
  useEffect(() => {
    const element = dialog.current;
    element?.showModal();
    return () => element?.close();
  }, []);
  useEffect(() => {
    let active = true;
    void prepareFirmwareUpdate(deviceId, version).then((value) => { if (active) setPlan(value); })
      .catch((reason: unknown) => { if (active) setError(String(reason)); })
      .finally(() => { if (active) setBusy(null); });
    return () => { active = false; };
  }, [deviceId, version]);
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  const expired = !!plan && now > plan.payload.expiresAtUnixSeconds * 1000;
  const target = plan?.payload.target;
  async function verify() {
    if (!plan || operation.current || expired || !firstConfirmed) return;
    operation.current = true; setBusy("verify"); setError(null);
    try {
      const report = await runFirmwareDryRun(plan.planId);
      if (report.verified && report.dryRun && report.planId === plan.planId) setVerified(true);
      else throw new Error("管理员复核结果无效");
    } catch (reason) { setError(String(reason)); }
    finally { operation.current = false; setBusy(null); }
  }
  async function install() {
    if (!plan || !verified || !secondConfirmed || operation.current || expired || failed || result) return;
    operation.current = true; setBusy("install"); setError(null); setEvents([]);
    try {
      setResult(await installDeviceFirmware(plan.planId, (event) => setEvents((previous) => [...previous.slice(-79), event])));
    } catch (reason) {
      setError(`${String(reason)}。固件操作可能已部分执行，请重新检查设备状态；不要直接重复刷写。`);
      setFailed(true); setVerified(false);
    } finally {
      operation.current = false; setBusy(null); onUpdated();
    }
  }
  return <dialog ref={dialog} className="firmware-dialog" aria-labelledby="firmware-dialog-title" onCancel={(event) => { event.preventDefault(); if (!busy) onClose(); }}>
    <header><h2 id="firmware-dialog-title">更新所选设备固件</h2><button className="close-button" aria-label="关闭" disabled={!!busy} onClick={onClose}>×</button></header>
    {busy === "plan" && <p role="status">正在检查并锁定设备和版本…</p>}
    {target && <>
      <dl className="local-deb-facts"><div><dt>设备</dt><dd>{target.deviceName}</dd></div><div><dt>版本</dt><dd>{target.installedVersion} → {target.targetVersion}</dd></div><div><dt>固件来源</dt><dd title={target.location}>{target.remoteId}</dd></div><div><dt>设备 ID</dt><dd title={target.deviceId}>{target.deviceId}</dd></div></dl>
      <p>固件更新期间请勿断电、拔出设备或强制退出。更新可能影响磁盘解密和启动验证，请提前保存工作、备份重要数据并准备好磁盘加密恢复密钥。</p>
      {target.requiresAc && <p>此设备要求连接电源。</p>}
      {(target.needsReboot || target.needsShutdown) && <p>更新后需要{target.needsShutdown ? "关机" : "重启"}。UManager 不会自动重启或关机。</p>}
      <p className="firmware-summary">计划有效至 {new Date(plan!.payload.expiresAtUnixSeconds * 1000).toLocaleTimeString()}</p>
      {!verified && !result && !failed && <>
        <label className="plan-confirmation"><input type="checkbox" checked={firstConfirmed} disabled={!!busy || expired} onChange={(event) => setFirstConfirmed(event.target.checked)}/>我已核对设备与目标版本，先进行管理员复核。</label>
        <button className="secondary-button" disabled={!!busy || expired || !firstConfirmed} onClick={() => void verify()}>{busy === "verify" ? "等待管理员授权与复核…" : "授权并复核（不安装）"}</button>
      </>}
      {verified && !result && <>
        <p className="firmware-notice">管理员复核通过，尚未下载或安装固件。设备状态将在执行前再次检查。</p>
        <label className="plan-confirmation"><input type="checkbox" checked={secondConfirmed} disabled={!!busy || expired} onChange={(event) => setSecondConfirmed(event.target.checked)}/>我已做好准备，确认将上述设备更新到 {target.targetVersion}。</label>
        <button className="primary-button" disabled={!!busy || expired || !secondConfirmed} onClick={() => void install()}>{busy === "install" ? "正在更新，请勿断电…" : `再次确认并更新到 ${target.targetVersion}`}</button>
      </>}
    </>}
    {busy === "install" && <p role="status">正在由系统 fwupd 下载并安装固件，请保持电源连接。完成前无法取消。</p>}
    {events.length > 0 && <pre className="firmware-log" aria-label="固件操作日志">{events.map((event) => event.message).join("\n")}</pre>}
    {expired && !result && busy !== "install" && <p className="firmware-error" role="alert">计划已过期，请关闭后重新检查并授权。</p>}
    {error && <p className="firmware-error" role="alert">{error}</p>}
    {result && <p className="firmware-notice" role="status">{result.message}</p>}
    <footer><button className="secondary-button" disabled={!!busy} onClick={onClose}>{result || failed ? "完成并返回设备列表" : "取消"}</button></footer>
  </dialog>;
}
