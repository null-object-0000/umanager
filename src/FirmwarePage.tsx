import { useEffect, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import rehypeRaw from "rehype-raw";
import rehypeSanitize from "rehype-sanitize";
import { getFirmwareState, refreshFirmwareMetadata } from "./api";
import type { FirmwareState } from "./types";
import { FirmwareDialog } from "./FirmwareDialog";

const problemLabels: Record<string, string> = {
  "require-ac-power": "需要连接电源", "power-too-low": "电量不足",
  "firmware-locked": "固件已锁定", "system-power-too-low": "系统电量不足",
  "update-pending": "更新待完成", "device-is-unreachable": "设备暂不可访问",
};

export function FirmwarePage() {
  const [state, setState] = useState<FirmwareState | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshStage, setRefreshStage] = useState<"refresh" | "check" | null>(null);
  const [refreshedAt, setRefreshedAt] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const refreshing = useRef(false);
  const mounted = useRef(false);
  const [error, setError] = useState<string | null>(null);
  const [checkedAt, setCheckedAt] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);
  const [selected, setSelected] = useState<{ deviceId: string; version: string } | null>(null);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  useEffect(() => {
    let active = true;
    setLoading(true);
    setError(null);
    setNotice(null);
    void getFirmwareState().then((result) => {
      if (active) { setState(result); setCheckedAt(new Date().toLocaleTimeString()); }
    }).catch((reason: unknown) => {
      if (active) setError(String(reason));
    }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [revision]);
  const busy = loading || refreshStage !== null;
  async function refreshMetadata() {
    if (busy || refreshing.current || selected) return;
    refreshing.current = true;
    setRefreshStage("refresh"); setError(null); setNotice(null);
    let refreshed = false;
    try {
      await refreshFirmwareMetadata();
      refreshed = true;
      if (!mounted.current) return;
      setRefreshedAt(new Date().toLocaleTimeString());
      setRefreshStage("check");
      const result = await getFirmwareState();
      if (!mounted.current) return;
      setState(result); setCheckedAt(new Date().toLocaleTimeString());
      setNotice(result.available && result.updatesChecked
        ? "元数据已联网刷新，设备与可用更新已重新检查；未执行固件安装。"
        : "元数据已联网刷新，但设备或更新检查未完成，请查看下方提示。");
    } catch (reason) {
      if (mounted.current) setError(refreshed ? `元数据已刷新，但重新检查失败：${String(reason)}` : String(reason));
    } finally {
      refreshing.current = false;
      if (mounted.current) setRefreshStage(null);
    }
  }
  const updateCount = state?.devices.filter((device) => device.releases.length > 0).length ?? 0;
  return <main className="workspace firmware-workspace">
    <header className="workspace-header">
      <div><h1>固件更新</h1><p>查看 BIOS、存储设备及外设的固件状态</p></div>
      <div className="header-actions">
        <button className="secondary-button" disabled={busy || !!selected} onClick={() => setRevision((value) => value + 1)}>{loading ? "正在检查…" : "重新检查"}</button>
        <button className="primary-button" disabled={busy || !!selected || state?.available === false} onClick={() => void refreshMetadata()}>{refreshStage === "refresh" ? "正在联网刷新…" : refreshStage === "check" ? "刷新后正在检查…" : "联网刷新元数据"}</button>
      </div>
    </header>
    <div className="firmware-notice">「重新检查」读取当前缓存；「联网刷新元数据」从已启用的固件源获取最新信息后重新检查，不启用测试源，也不安装固件。安装仍需单独选择设备、管理员复核与二次确认。</div>
    {loading && <p role="status">正在读取设备及可用更新…</p>}
    {refreshStage && <p role="status">{refreshStage === "refresh" ? "正在联网刷新已启用的固件源，仅下载元数据与签名…" : "元数据刷新完成，正在重新检查设备及可用更新…"}</p>}
    {notice && <p className="firmware-notice" role="status">{notice}</p>}
    {refreshedAt && <p className="firmware-summary">上次联网刷新：{refreshedAt}</p>}
    {error && <div className="firmware-error" role="alert">{error}{state && "。下方保留上次检查结果。"}</div>}
    {state && !state.available && <section className="firmware-card"><h2>未检测到 fwupd</h2><p>需要系统提供 fwupd 才能读取固件信息。可通过 Ubuntu 的 APT 安装 fwupd，无需安装 Snap。</p></section>}
    {state?.warnings.map((warning) => <p className="firmware-error" role="alert" key={warning}>{warning}</p>)}
    {state?.available && <>
      <p className="firmware-summary">检测到 {state.devices.length} 个设备 · {state.updatesChecked ? `${updateCount} 个设备有可用更新` : "更新检查未完成"}{checkedAt && ` · 检查于 ${checkedAt}`}</p>
      {!state.devices.length && <section className="firmware-card"><h2>未检测到设备</h2><p>fwupd 暂未返回设备信息，这不表示硬件发生故障。</p></section>}
      {state.updatesChecked && !updateCount && state.devices.length > 0 && <p className="firmware-notice">当前缓存的元数据未报告可用更新；这不代表所有设备都支持 fwupd，或已安装厂商最新固件。</p>}
      <div className="firmware-devices">{state.devices.map((device) => <article className="firmware-card" key={device.id}>
        <div className="firmware-card-heading"><h2>{device.name}</h2><span className={device.releases.length ? "firmware-update-badge" : "firmware-status"}>{device.releases.length ? "有可用更新" : !device.updatable ? "未开放更新能力" : !state.updatesChecked ? "更新状态未知" : device.supported ? "未发现更新" : "当前源未标记为受支持"}</span></div>
        <dl><div><dt>厂商</dt><dd>{device.vendor ?? "未知"}</dd></div><div><dt>当前版本</dt><dd>{device.version ?? "未提供"}</dd></div></dl>
        <div className="firmware-requirements">{device.requiresAc && <span>更新需要连接电源</span>}{device.needsShutdown ? <span>更新需要关机</span> : device.needsReboot && <span>更新需要重启</span>}</div>
        {device.problems.length > 0 && <p className="firmware-error">设备限制：{device.problems.map((problem) => problemLabels[problem] ?? problem).join("、")}</p>}
        {device.updateError && <p className="firmware-error">更新提示：{device.updateError}</p>}
        {device.releases.map((release, index) => <section className="firmware-release" key={`${release.version}-${index}`}>
          <h3>可用版本 {release.version}</h3>
          {release.name && <p>{release.name}</p>}{release.summary && <p>{release.summary}</p>}
          {release.description && <details><summary>更新说明</summary><ReactMarkdown rehypePlugins={[rehypeRaw, rehypeSanitize]} components={{ a: ({ children }) => <span>{children}</span>, img: () => null }}>{release.description}</ReactMarkdown></details>}
          <button className="secondary-button" disabled={busy || !!error || !state.updatesChecked || !device.updatable || !device.supported || device.problems.length > 0 || !!device.updateError || !release.flags.includes("trusted-metadata") || !release.remoteId || !release.checksum.some((value) => /^[a-fA-F0-9]{64}$/.test(value))} onClick={() => setSelected({ deviceId: device.id, version: release.version })}>更新此设备到 {release.version}</button>
          {(!release.flags.includes("trusted-metadata") || !release.remoteId || !release.checksum.some((value) => /^[a-fA-F0-9]{64}$/.test(value))) && <p className="firmware-error">此版本缺少可信来源或 SHA-256 信息，暂不能安装。</p>}
        </section>)}
      </article>)}</div>
    </>}
    {selected && <FirmwareDialog deviceId={selected.deviceId} version={selected.version} onClose={() => setSelected(null)} onUpdated={() => setRevision((value) => value + 1)}/>}
  </main>;
}
