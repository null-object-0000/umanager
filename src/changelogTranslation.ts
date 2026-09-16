/// 更新日志「翻译 + 归纳」的展示逻辑。这里的函数都是纯函数，便于单测覆盖；
/// 真正的请求与缓存由 `api.ts` / Rust 侧的 `translation.rs` 负责。

/// 中文用户看到纯英文更新日志时才提供翻译：有拉丁文字、且完全没有 CJK 字符。
export function looksEnglish(text: string): boolean {
  if (!text) return false;
  const hasCjk = /[\u3040-\u30ff\u3400-\u4dbf\u4e00-\u9fff\uac00-\ud7af]/.test(text);
  const hasLatin = /[A-Za-z]{3,}/.test(text);
  return hasLatin && !hasCjk;
}

export interface TranslationActionState {
  translating: boolean;
  /// 当前是否正在看译文（false = 看原文）。
  showTranslated: boolean;
  /// 是否已经有一份可展示的结果（本次翻译的，或上次落到本地的）。
  hasTranslation: boolean;
}

/// 主按钮文案。首次点击是「翻译并总结」——一次请求同时拿到译文和更新重点。
export function translateButtonLabel(state: TranslationActionState): string {
  if (state.translating) return "翻译中…";
  if (state.showTranslated) return "查看原文";
  return state.hasTranslation ? "查看译文" : "翻译并总结";
}

/// 是否显示「重新翻译」：正在看译文且有结果时，给用户一个绕过本地缓存的入口。
export function canRetranslate(state: TranslationActionState): boolean {
  return state.hasTranslation && state.showTranslated && !state.translating;
}

/// 「上次翻译」来源说明。`dateText` 由调用方格式化（与页面其它时间展示保持一致），
/// 缺失时只显示模型名。
export function formatCachedOrigin(model: string, dateText: string | null): string {
  const parts = ["已展示上次翻译结果"];
  if (dateText) parts.push(dateText);
  const trimmedModel = model.trim();
  if (trimmedModel) parts.push(trimmedModel);
  return parts.join(" · ");
}

/// 译文已经拿到、但总结请求失败时的兜底提示（总结失败不影响译文展示）。
export function shouldWarnMissingSummary(
  summary: string | null,
  state: { translating: boolean; showTranslated: boolean; hasTranslation: boolean },
): boolean {
  return state.showTranslated && state.hasTranslation && !state.translating && !summary?.trim();
}
