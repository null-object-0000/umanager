import { useEffect, useMemo, useState } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { copyClipboardEntry, dragClipboardImage, getClipboardHistoryRevision, hideClipboardPanel, listClipboardHistory, onClipboardHistoryChanged } from "./api";
import type { ClipboardEntry } from "./types";

export default function ClipboardPanel() {
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
    onClipboardHistoryChanged((list) => { if (active) setEntries(list); })
      .then((fn) => { if (active) unlisten = fn; else fn(); }).catch(() => {});
    // 面板在短暂隐藏后可直接复用，恢复焦点时同步历史。
    let unlistenFocus: (() => void) | null = null;
    getCurrentWebviewWindow()
      .onFocusChanged(({ payload }) => { if (payload) void refresh(); })
      .then((fn) => { if (active) unlistenFocus = fn; else fn(); })
      .catch(() => {});
    const onKey = (event: KeyboardEvent) => { if (event.key === "Escape") void hideClipboardPanel(); };
    window.addEventListener("keydown", onKey);
    // 隐藏期间 WebKitGTK 可能挂起整个页面，导致事件/焦点回调都不可靠。
    // 这里用 visibilitychange 立即补拉 + 1s 轻量版本号轮询兜底，保证面板恢复可见时
    // 能及时刷新，且不会频繁传输带缩略图的完整列表。
    const onVisibility = () => { if (document.visibilityState === "visible") void refresh(); };
    document.addEventListener("visibilitychange", onVisibility);
    const interval = window.setInterval(() => {
      if (document.visibilityState === "visible") void refresh();
    }, 1000);
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
      <label className="search-box clip-panel-search"><svg className="ui-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><circle cx="10.5" cy="10.5" r="7.5"/><path d="m16 16 5 5"/></svg><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索" autoFocus/></label>
    </div>
    <div className="clip-panel-list">
      {visible.length === 0 && <div className="clip-panel-empty">{entries.length === 0 ? "还没有记录" : "无匹配"}</div>}
      {visible.map((entry) => entry.kind === "image"
        ? <div className={`clip-panel-item image ${copiedId === entry.id ? "copied" : ""}`} key={entry.id} draggable
            onClick={() => void copy(entry)} title="点击复制图片，或拖到聊天窗口发送文件"
            onDragStart={(event) => { event.preventDefault(); dragClipboardImage(entry.id).catch((reason) => console.error("图片拖拽失败", reason)); }}>
            <img className="clip-panel-thumb" loading="lazy" decoding="async" draggable={false} src={entry.imagePreview ?? ""} alt="剪贴板图片"/>
            <span className="clip-panel-dim">{entry.imageWidth ?? 0}×{entry.imageHeight ?? 0}</span>
          </div>
        : <div className={`clip-panel-item text ${copiedId === entry.id ? "copied" : ""}`} key={entry.id} onClick={() => void copy(entry)} title="点击复制">
            <span className="clip-panel-text">{entry.text?.slice(0, 512)}</span>
          </div>)}
    </div>
  </div>;
}
