// 卡片页脚里的「当前版本发布时间」。商店列表按「最近更新」排序时，这个日期就是
// 排序依据；同时它也回答「最新的版本是啥时候发布的」。时间来自签名 feed，分三种
// 来源（官方发布时间 / 官方安装包更新时间 / 首次采集到该版本），用 title 说明，
// 避免把「采集推断」当成官方发布时间。
import type { VersionUpdatedAtSource } from "./types";

const sourceText: Record<VersionUpdatedAtSource, string> = {
  official: "官方发布时间",
  serverModified: "官方安装包更新时间",
  observed: "首次采集到该版本的时间",
};

export function formatUpdatedDate(unixSeconds: number | null | undefined) {
  if (unixSeconds == null) return null;
  return new Date(unixSeconds * 1000).toLocaleDateString("zh-CN", { year: "numeric", month: "2-digit", day: "2-digit" });
}

export function updatedAtTooltip(unixSeconds: number | null | undefined, source: VersionUpdatedAtSource | null | undefined) {
  if (unixSeconds == null) return undefined;
  const label = source ? sourceText[source] : "版本发布时间";
  const exact = new Date(unixSeconds * 1000).toLocaleString("zh-CN", { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" });
  return `${label} · ${exact}`;
}

export function VersionDate({ seconds, source }: { seconds: number | null | undefined; source: VersionUpdatedAtSource | null | undefined }) {
  const date = formatUpdatedDate(seconds);
  if (!date) return null;
  return <span className="app-card-updated" title={updatedAtTooltip(seconds, source)}>{date}</span>;
}
