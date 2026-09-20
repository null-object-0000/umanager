/// 更新日志「翻译 / 归纳」的展示逻辑。这里的函数都是纯函数，便于单测覆盖；
/// 真正的请求与缓存由 `api.ts` / Rust 侧的 `translation.rs` 负责。

import type { ChangelogAiMode } from "./types";

/// 英文判定：有拉丁文字、且完全没有 CJK 字符。只有这种更新日志需要翻译。
export function looksEnglish(text: string): boolean {
  if (!text) return false;
  const hasCjk = /[\u3040-\u30ff\u3400-\u4dbf\u4e00-\u9fff\uac00-\ud7af]/.test(text);
  const hasLatin = /[A-Za-z]{3,}/.test(text);
  return hasLatin && !hasCjk;
}

/// 更新日志的 AI 入口模式：英文 → 翻译成中文（顺带归纳「更新重点」）；其余语言
/// （含中英混排）→ 只归纳「更新重点」，不翻译、不覆盖原文。没有正文时不提供入口。
export function aiMode(text: string | null | undefined): ChangelogAiMode | null {
  const trimmed = text?.trim() ?? "";
  if (!trimmed) return null;
  return looksEnglish(trimmed) ? "translate" : "summarize";
}

export interface AiActionState {
  mode: ChangelogAiMode;
  /// 请求是否正在进行。
  busy: boolean;
  /// 当前是否正在展示 AI 结果（译文 / 更新重点）。
  showing: boolean;
  /// 是否已有一份可展示的结果（本次请求的，或上次落到本地的）。
  hasResult: boolean;
}

/// 主按钮文案。首次点击在英文日志上是「翻译并总结」（一次请求同时拿到译文和更新
/// 重点），在中文等日志上是「归纳总结」（只有更新重点，原文始终在下方）。
export function aiButtonLabel(state: AiActionState): string {
  if (state.busy) return state.mode === "translate" ? "翻译中…" : "归纳中…";
  if (state.showing) return state.mode === "translate" ? "查看原文" : "收起总结";
  if (state.hasResult) return state.mode === "translate" ? "查看译文" : "展开总结";
  return state.mode === "translate" ? "翻译并总结" : "归纳总结";
}

/// 是否显示「重新翻译 / 重新归纳」：正在看结果且有结果时，给用户一个绕过本地缓存
/// 的入口。
export function canRegenerate(state: AiActionState): boolean {
  return state.hasResult && state.showing && !state.busy;
}

export function regenerateButtonLabel(mode: ChangelogAiMode): string {
  return mode === "translate" ? "重新翻译" : "重新归纳";
}

/// 「上次翻译 / 上次归纳」来源说明。`dateText` 由调用方格式化（与页面其它时间展示
/// 保持一致），缺失时只显示模型名。
export function formatCachedOrigin(mode: ChangelogAiMode, model: string, dateText: string | null): string {
  const parts = [mode === "translate" ? "已展示上次翻译结果" : "已展示上次归纳结果"];
  if (dateText) parts.push(dateText);
  const trimmedModel = model.trim();
  if (trimmedModel) parts.push(trimmedModel);
  return parts.join(" · ");
}

/// 译文已经拿到、但总结请求失败时的兜底提示（总结失败不影响译文展示）。归纳模式下
/// 拿不到总结就是整次请求失败，会走错误提示，不再重复提示。
export function shouldWarnMissingSummary(summary: string | null, state: AiActionState): boolean {
  return state.mode === "translate" && state.showing && state.hasResult && !state.busy && !summary?.trim();
}
