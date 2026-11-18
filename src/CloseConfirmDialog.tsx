import { useEffect, useRef } from "react";
import { closePrompt } from "./closeBehavior";
import type { CloseSummary } from "./closeBehavior";

export function CloseConfirmDialog({ summary, onConfirm, onCancel }: {
  summary: CloseSummary;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => { ref.current?.showModal(); return () => ref.current?.close(); }, []);
  const prompt = closePrompt(summary);
  return <dialog ref={ref} className="windows-confirm close-confirm" aria-labelledby="close-confirm-title" onCancel={(event) => { event.preventDefault(); onCancel(); }}>
    <h2 id="close-confirm-title">{prompt.title}</h2>
    <p>{prompt.body}</p>
    <p className="windows-note">想让任务继续运行，请选择「继续使用」，然后最小化窗口。</p>
    <div className="windows-actions">
      <button className="secondary-button" onClick={onCancel} autoFocus>继续使用</button>
      <button className="primary-button" onClick={onConfirm}>退出 UManager</button>
    </div>
  </dialog>;
}
