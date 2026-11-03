import { useEffect, useRef, useState } from "react";
import { batchActive, batchItemLabels, batchStatusLabels, canConfirmBatch, reviewExpiresAt, unfinishedIds } from "./batchTasks";
import type { BatchCandidate } from "./batchTasks";
import { batchPreview, cancelBatchUpdate, executeBatchUpdate, listBatchTasks, onBatchTasksChanged, reviewBatchUpdate, startBatchUpdate } from "./batchApi";
import { formatBytes } from "./DownloadProgress";
import type { BatchSnapshot, BatchTask, DownloadJob } from "./types";

export function useBatchTasks(onChanged: () => void) {
  const [snapshot, setSnapshot] = useState<BatchSnapshot>({ revision: -1, batches: [] });
  const [error, setError] = useState<string | null>(null);
  const latest = useRef(snapshot);
  const onChangedRef = useRef(onChanged);
  onChangedRef.current = onChanged;
  const apply = (next: BatchSnapshot) => {
    if (next.revision < latest.current.revision) return;
    const previous = latest.current;
    latest.current = next; setSnapshot(next); setError(null);
    if (previous.revision >= 0 && next.batches.some((b) => {
      const before = previous.batches.find((old) => old.id === b.id);
      return before && before.status === "installing" && b.status !== "installing";
    })) onChangedRef.current();
  };
  const refresh = async () => { try { apply(await listBatchTasks()); } catch (reason) { setError(String(reason)); } };
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    // Register before reading the first snapshot; revisions discard late reads.
    void onBatchTasksChanged((next) => { if (!disposed) apply(next); }).then((stop) => {
      if (disposed) { stop(); return; }
      unlisten = stop;
      void refresh();
    }).catch((reason) => { if (!disposed) setError(String(reason)); });
    const focus = () => { if (!disposed) void refresh(); };
    window.addEventListener("focus", focus);
    return () => { disposed = true; unlisten?.(); window.removeEventListener("focus", focus); };
  }, []);
  return { snapshot, error, apply, refresh };
}

export function BatchUpdatePicker({ candidates, skipped, onClose, onStarted }: {
  candidates: BatchCandidate[]; skipped: number; onClose: () => void; onStarted: (snapshot: BatchSnapshot) => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [selected, setSelected] = useState(() => candidates.map((c) => c.applicationId));
  const [busy, setBusy] = useState(false);
  const working = useRef(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => { const element = dialog.current; element?.showModal(); return () => element?.close(); }, []);
  const start = async () => {
    if (working.current || !selected.length) return;
    working.current = true; setBusy(true); setError(null);
    try { onStarted(await startBatchUpdate(selected)); }
    catch (reason) { setError(String(reason)); }
    finally { working.current = false; setBusy(false); }
  };
  return <dialog ref={dialog} className="batch-picker" aria-labelledby="batch-picker-title" onCancel={(event) => { event.preventDefault(); if (!busy) onClose(); }}>
    <header><div><h2 id="batch-picker-title">选择本次更新的软件</h2><p>先下载并校验；完成后到任务中心授权复核，再次确认安装。</p></div><button className="ghost-link" aria-label="关闭选择清单" disabled={busy} onClick={onClose}>×</button></header>
    <p className="batch-scope">支持已安装的受管 .deb。UManager 自更新、CLI 工具、Windows 应用和固件需单独处理。</p>
    {skipped > 0 && <p role="status">重新检查后，{skipped} 项已无可用更新或不再符合条件，本次不纳入；原任务记录保留。</p>}
    {candidates.length > 0 ? <>
      <label className="batch-select-all"><input type="checkbox" checked={selected.length === candidates.length} disabled={busy} onChange={(event) => setSelected(event.target.checked ? candidates.map((c) => c.applicationId) : [])}/>全选 · 已选 {selected.length} / {candidates.length}</label>
      <ul className="batch-selection-list">{candidates.map(({ applicationId, package: pkg }) => <li key={applicationId}><label><input type="checkbox" checked={selected.includes(applicationId)} disabled={busy} onChange={(event) => setSelected((ids) => event.target.checked ? [...ids, applicationId] : ids.filter((id) => id !== applicationId))}/><span><strong>{pkg.displayName}</strong><small>{pkg.installedVersion} → {pkg.candidateVersion}</small><small>{pkg.packageName} · {pkg.architecture}</small></span></label></li>)}</ul>
    </> : <div className="dev-empty">重新检查后，没有符合条件的 .deb 更新。</div>}
    {error && <p className="batch-error" role="alert">{error}</p>}
    <footer><button className="secondary-button" disabled={busy} onClick={onClose}>取消</button><button className="primary-button" disabled={busy || !selected.length} onClick={() => void start()}>{busy ? "正在建立批量任务…" : `下载所选 ${selected.length} 项`}</button></footer>
  </dialog>;
}

function BatchCard({ batch, jobs, onSnapshot, onRetry }: {
  batch: BatchTask; jobs: DownloadJob[]; onSnapshot: (snapshot: BatchSnapshot) => void; onRetry: (batch: BatchTask) => void;
}) {
  const [busy, setBusy] = useState(false);
  const working = useRef(false);
  const [confirmed, setConfirmed] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [now, setNow] = useState(Date.now() / 1000);
  useEffect(() => { setConfirmed(false); }, [batch.reviewId]);
  useEffect(() => {
    if (batch.status !== "awaitingConfirmation") return;
    setNow(Date.now() / 1000);
    const timer = window.setInterval(() => setNow(Date.now() / 1000), 1000);
    return () => window.clearInterval(timer);
  }, [batch.status, batch.reviewId]);
  const action = async (operation: () => Promise<BatchSnapshot>) => {
    if (working.current) return;
    working.current = true; setBusy(true); setError(null);
    try { onSnapshot(await operation()); } catch (reason) { setError(String(reason)); try { onSnapshot(await listBatchTasks()); } catch { /* Keep the action error visible. */ } }
    finally { working.current = false; setBusy(false); }
  };
  const successes = batch.items.filter((i) => i.status === "succeeded").length;
  const ready = batch.items.filter((i) => i.status === "ready" || i.status === "reviewed").length;
  const expired = batch.status === "awaitingConfirmation" && !canConfirmBatch(batch, now);
  const expiry = reviewExpiresAt(batch);
  return <article className={`batch-card batch-${batch.status}`}>
    <header className="batch-card-header"><div><h2>{new Date(batch.createdAtUnixSeconds * 1000).toLocaleString("zh-CN")} 的更新</h2><p>共 {batch.items.length} 项 · 成功 {successes} 项</p></div><span className="batch-status" role="status">{batchStatusLabels[batch.status]}</span></header>
    <p className="batch-message">{batch.message}</p>
    <ol className="batch-items">{batch.items.map((item) => {
      const job = jobs.find((j) => j.jobId === item.downloadJobId);
      const progress = item.status === "downloading" ? job?.progress : null;
      return <li key={item.applicationId} className={`batch-item-${item.status}`}>
        <div className="batch-item-top"><strong>{item.displayName}</strong><span>{batchItemLabels[item.status]}</span></div>
        <p className="batch-version">{item.installedVersion} → {item.targetVersion}</p>
        {progress && <div className="batch-progress"><progress max={progress.totalBytes || 1} value={progress.transferredBytes} aria-label={`${item.displayName} 下载进度`}/><span>{formatBytes(progress.transferredBytes)} / {formatBytes(progress.totalBytes)}{progress.bytesPerSecond > 0 ? ` · ${formatBytes(progress.bytesPerSecond)}/s` : ""}</span></div>}
        {item.error && <p className="batch-error">{item.error}</p>}
        <details className="batch-item-details"><summary>来源校验与操作日志</summary><dl><dt>软件包</dt><dd>{item.packageName} · {item.architecture}</dd><dt>安装包</dt><dd>{formatBytes(item.size)}</dd><dt>下载来源</dt><dd>{item.downloadUrl}</dd><dt>SHA-256</dt><dd>{item.sha256}</dd>{item.planId && <><dt>计划 ID</dt><dd>{item.planId}</dd></>}</dl>{item.logs.length > 0 ? <pre>{item.logs.join("\n")}</pre> : <p>尚无安装日志。</p>}</details>
      </li>;
    })}</ol>
    {batch.status === "awaitingReview" && <div className="batch-actions"><p>本次将复核 {ready} 项；复核阶段不安装。任何一项复核失败，本批次不会开始安装。</p><button className="primary-button" disabled={busy} onClick={() => void action(() => reviewBatchUpdate(batch.id))}>授权并复核 {ready} 项（不安装）</button></div>}
    {batch.status === "awaitingConfirmation" && <div className="batch-confirmation">
      {expired ? <p className="batch-error" role="status">复核计划已过期，请重新授权复核。</p> : <p>复核通过，清单有效至 {expiry ? new Date(expiry * 1000).toLocaleTimeString("zh-CN") : "—"}。安装串行执行，失败后停止后续项目。</p>}
      <label><input type="checkbox" checked={confirmed} disabled={busy || expired} onChange={(event) => setConfirmed(event.target.checked)}/>我已核对以上 {ready} 项更新，确认执行这些计划；理解安装会运行厂商的软件包脚本。</label>
      <div className="batch-actions"><button className="primary-button" disabled={busy || !confirmed || !canConfirmBatch(batch, now)} onClick={() => { setConfirmed(false); void action(() => executeBatchUpdate(batch.id, batch.reviewId!)); }}>再次确认并安装 {ready} 项</button><button className="secondary-button" disabled={busy} onClick={() => void action(() => reviewBatchUpdate(batch.id))}>重新授权复核</button></div>
      <details className="batch-item-details"><summary>本次复核清单摘要</summary><code>{batch.reviewId}</code></details>
    </div>}
    {["preparing", "downloading", "awaitingReview", "awaitingConfirmation"].includes(batch.status) && <button className="ghost-link" disabled={busy} onClick={() => void action(() => cancelBatchUpdate(batch.id))}>结束本批次（保留下载）</button>}
    {(batch.status === "paused" || batch.status === "interrupted") && <button className="ghost-link" disabled={busy} onClick={() => void action(() => cancelBatchUpdate(batch.id))}>结束跟进（保留记录）</button>}
    {!batchActive(batch.status) && unfinishedIds(batch).length > 0 && <div className="batch-actions"><button className="secondary-button" disabled={busy} onClick={() => onRetry(batch)}>重新检查未完成项…</button><p>保留成功记录；重新检查后选择仍需更新的项目，建立新任务并重新确认。</p></div>}
    {(batch.status === "installing" || batch.status === "reviewing") && <p className="batch-running" role="status">可以离开此页，任务仍在后台运行。{batch.status === "installing" ? "安装期间请勿退出程序或关闭电脑。" : "等待系统授权时请完成管理员验证。"}</p>}
    {error && <p className="batch-error" role="alert">{error}</p>}
  </article>;
}

export function TasksPage({ snapshot, error, jobs, onSnapshot, onRefresh, onRetry, onDownloads }: {
  snapshot: BatchSnapshot; error: string | null; jobs: DownloadJob[]; onSnapshot: (snapshot: BatchSnapshot) => void;
  onRefresh: () => void; onRetry: (batch: BatchTask) => void; onDownloads: () => void;
}) {
  const [filter, setFilter] = useState<"all" | "active" | "attention" | "completed">("all");
  const ordered = [...snapshot.batches].reverse();
  const visible = ordered.filter((b) => filter === "all" || (filter === "active" ? batchActive(b.status) : filter === "attention" ? b.status === "paused" || b.status === "interrupted" : b.status === "completed"));
  return <main className="workspace tasks-workspace">
    <header className="workspace-header"><div><h1>任务中心</h1><p>批量更新进度、确认清单与操作记录</p></div><div className="header-actions"><button className="secondary-button" onClick={onDownloads}>下载队列</button><button className="primary-button" onClick={onRefresh}>刷新记录</button></div></header>
    {batchPreview() && <p className="batch-preview">浏览器预览：授权与安装均为模拟，不会修改本机软件。</p>}
    <div className="filter-tabs segmented batch-filters" aria-label="筛选任务">{([ ["all", "全部"], ["active", "进行中"], ["attention", "需处理"], ["completed", "已完成"] ] as const).map(([id, label]) => <button key={id} className={filter === id ? "active" : ""} onClick={() => setFilter(id)}>{label}</button>)}</div>
    {error && <p className="batch-error" role="alert">{error}</p>}
    {snapshot.storageError && <p className="batch-error" role="alert">任务记录保存失败，已停止后续操作：{snapshot.storageError}</p>}
    {snapshot.revision < 0 && !error ? <p className="dev-empty">正在读取任务记录…</p> : visible.length === 0 && <div className="software-panel dev-empty">{ordered.length ? "没有符合筛选条件的任务。" : "暂无批量更新任务。到「更新」页选择「批量更新 .deb」开始。"}</div>}
    {visible.map((batch) => <BatchCard key={batch.id} batch={batch} jobs={jobs} onSnapshot={onSnapshot} onRetry={onRetry}/>)}
    <p className="batch-history-note">本机保存最近 100 批更新记录。重新启动后，未完成任务需要重新检查与确认；历史记录不会自动触发安装。</p>
  </main>;
}
