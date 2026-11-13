import { useEffect, useRef } from "react";
import { closePrompt } from "./closeBehavior";
import type { CloseSummary } from "./closeBehavior";

/**
 * 托盘不可用时，「×」意味着真正退出。这里在退出前如实交代还剩多少没结束的任务，
 * 并给一次取消的机会——之前的问题正是窗口直接消失、任务中心也跟着没了。
 */
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
    <p className="windows-note">关闭窗口就是退出 UManager（含后台下载与更新）。想留在后台运行，请用系统托盘的「显示 / 退出」菜单。</p>
    <div className="windows-actions">
      <button className="secondary-button" onClick={onCancel} autoFocus>继续使用</button>
      <button className="primary-button" onClick={onConfirm}>退出 UManager</button>
    </div>
  </dialog>;
}
