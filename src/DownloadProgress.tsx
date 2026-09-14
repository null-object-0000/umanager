// 下载进度展示：软件卡片圆环与详情抽屉 hero 进度。Debian 软件包与 Windows（Wine）
// 安装包共用同一套组件与同一份 DownloadProgress 载荷，两种软件的下载体验保持一致
// （点「更新 / 获取」后按钮位置变成进度环，抽屉顶部显示同一个环，不重复渲染进度）。
import type { DownloadProgress } from "./types";

export function formatBytes(bytes: number) {
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MiB`;
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
