// 下载进度展示：卡片圆环、抽屉 hero 进度与抽屉内进度条。Debian 软件包与 Windows
// （Wine）安装包共用同一套组件与同一份 DownloadProgress 载荷，两种软件的下载体验
// 保持一致（点「更新 / 获取」后按钮位置变成进度环）。
import type { DownloadProgress } from "./types";

export function formatBytes(bytes: number) {
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MiB`;
}

export function formatSpeed(bytesPerSecond: number) {
  if (bytesPerSecond >= 1024 * 1024) return `${(bytesPerSecond / 1024 / 1024).toFixed(1)} MiB/s`;
  return `${(bytesPerSecond / 1024).toFixed(0)} KiB/s`;
}

export function CardDownloadRing({ progress }: { progress: DownloadProgress }) {
  const percent = progress.totalBytes > 0 ? Math.min(100, Math.round(progress.transferredBytes / progress.totalBytes * 100)) : 0;
  const label = progress.phase === "verifying" ? "校验中" : `${percent}%`;
  return <span className="card-download-ring" style={{ background: `conic-gradient(var(--accent) ${percent * 3.6}deg, rgba(0,0,0,0.06) 0deg)` }} role="progressbar" aria-label={label} aria-valuenow={percent} aria-valuemin={0} aria-valuemax={100} title={label}><span>{percent}</span></span>;
}

export function HeroDownloadProgress({ progress }: { progress: DownloadProgress }) {
  const percent = progress.totalBytes > 0 ? Math.min(100, Math.round(progress.transferredBytes / progress.totalBytes * 100)) : 0;
  const label = progress.phase === "verifying" ? "正在校验…" : "正在下载…";
  return <div className="hero-download" role="progressbar" aria-label={label} aria-valuenow={percent} aria-valuemin={0} aria-valuemax={100}>
    <span className="hero-download-ring" style={{ background: `conic-gradient(var(--accent) ${percent * 3.6}deg, rgba(0,0,0,0.08) 0deg)` }}><span className="hero-download-ring-inner">{percent}%</span></span>
    <span className="hero-download-label">{label}</span>
    <span className="hero-download-stats">{formatBytes(progress.transferredBytes)} / {formatBytes(progress.totalBytes)}</span>
  </div>;
}

export function DownloadProgressCard({ progress, displayName }: { progress: DownloadProgress; displayName: string }) {
  const percent = Math.min(100, Math.round(progress.transferredBytes / progress.totalBytes * 100));
  return <div className="download-progress-card" aria-live="polite">
    <div className="download-progress-title"><strong>{progress.phase === "downloading" ? `正在下载 ${displayName}` : `正在校验 ${displayName} 安装包`}</strong><span>{percent}%</span></div>
    <div className="download-progress-track"><span style={{ width: `${percent}%` }}/></div>
    <div className="download-progress-stats"><span>{formatBytes(progress.transferredBytes)} / {formatBytes(progress.totalBytes)}</span><strong>{progress.phase === "downloading" ? formatSpeed(progress.bytesPerSecond) : "正在复核包信息与 SHA-256"}</strong></div>
  </div>;
}
