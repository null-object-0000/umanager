import { useEffect, useState } from "react";
import ReactMarkdown from "react-markdown";
import rehypeRaw from "rehype-raw";
import rehypeSanitize from "rehype-sanitize";
import { getFirmwareState } from "./api";
import type { FirmwareState } from "./types";
import { FirmwareDialog } from "./FirmwareDialog";

const problemLabels: Record<string, string> = {
  "require-ac-power": "需要连接电源", "power-too-low": "电量不足",
  "firmware-locked": "固件已锁定", "system-power-too-low": "系统电量不足",
  "update-pending": "更新待完成", "device-is-unreachable": "设备暂不可访问",
};

export function FirmwarePage() {
  const [state, setState] = useState<FirmwareState | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [checkedAt, setCheckedAt] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);
  const [selected, setSelected] = useState<{ deviceId: string; version: string } | null>(null);
  useEffect(() => {
    let active = true;
    setLoading(true);
    setError(null);
    void getFirmwareState().then((result) => {
      if (active) { setState(result); setCheckedAt(new Date().toLocaleTimeString()); }
    }).catch((reason: unknown) => {
      if (active) setError(String(reason));
    }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [revision]);
  const updateCount = state?.devices.filter((device) => device.releases.length > 0).length ?? 0;
  return <main className="workspace firmware-workspace">
    <header className="workspace-header">
      <div><h1>固件更新</h1><p>查看 BIOS、存储设备及外设的固件状态</p></div>
      <button className="secondary-button" disabled={loading} onClick={() => setRevision((value) => value + 1)}>{loading ? "正在检查…" : "重新检查"}</button>
    </header>
    <div className="firmware-notice">检查使用系统 fwupd 已缓存的元数据，不会自动安装更新。选择设备后，需管理员复核并再次确认才会下载和安装；不会自动重启电脑。</div>
    {loading && <p role="status">正在读取设备及可用更新…</p>}
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
          <button className="secondary-button" disabled={loading || !!error || !state.updatesChecked || !device.updatable || !device.supported || device.problems.length > 0 || !!device.updateError || !release.flags.includes("trusted-metadata") || !release.remoteId || !release.checksum.some((value) => /^[a-fA-F0-9]{64}$/.test(value))} onClick={() => setSelected({ deviceId: device.id, version: release.version })}>更新此设备到 {release.version}</button>
          {(!release.flags.includes("trusted-metadata") || !release.remoteId || !release.checksum.some((value) => /^[a-fA-F0-9]{64}$/.test(value))) && <p className="firmware-error">此版本缺少可信来源或 SHA-256 信息，暂不能安装。</p>}
        </section>)}
      </article>)}</div>
    </>}
    {selected && <FirmwareDialog deviceId={selected.deviceId} version={selected.version} onClose={() => setSelected(null)} onUpdated={() => setRevision((value) => value + 1)}/>}
  </main>;
}
