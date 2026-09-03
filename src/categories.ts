// 软件分类：纯展示层。分类只影响「软件」页的分组与筛选，绝不参与 helper
// 授权或任何信任判断。
//
// 分类以「id + label」形式存在，id 是稳定标识、label 是可改的显示名。
// 运行时优先使用签名 feed 下发的分类表与归属（可动态调整、无需发版）；
// feed 不可用或未携带分类时，回退到下方内置映射；未知 id 统一兜底「其他」。

import type { CategoryCatalog } from "./types";

export const UNKNOWN_CATEGORY = "其他";

// 内置兜底：与仓库根目录 feed-categories.json 保持一致。
const FALLBACK_CATEGORIES = [
  { id: "dev-tools", label: "开发工具" },
  { id: "ai-tools", label: "AI 工具" },
  { id: "browser", label: "浏览器" },
  { id: "chat", label: "社交通讯" },
  { id: "office", label: "办公效率" },
  { id: "network", label: "网络工具" },
  { id: "security", label: "安全工具" },
  { id: "media", label: "影音娱乐" },
];

const FALLBACK_DEB_BY_ID: Record<string, string> = {
  vscode: "dev-tools",
  "github-cli": "dev-tools",
  chatgpt: "ai-tools",
  "google-chrome": "browser",
  "microsoft-edge": "browser",
  wechat: "chat",
  qq: "chat",
  wemeet: "office",
  obsidian: "office",
  feishu: "office",
  wps: "office",
  dingtalk: "office",
  "tencent-docs": "office",
  flclash: "network",
  localsend: "network",
  bitwarden: "security",
  "qq-music": "media",
};

const FALLBACK_DEV_TOOL_BY_ID: Record<string, string> = {
  "claude-code": "ai-tools",
  opencode: "ai-tools",
  pi: "ai-tools",
  codex: "ai-tools",
  dsh: "ai-tools",
  hermes: "ai-tools",
  uv: "dev-tools",
  pnpm: "dev-tools",
};

function categoryLabel(catalog: CategoryCatalog | null, id: string | null | undefined): string {
  if (!id) return UNKNOWN_CATEGORY;
  const categories = catalog?.categories ?? FALLBACK_CATEGORIES;
  return categories.find((category) => category.id === id)?.label ?? UNKNOWN_CATEGORY;
}

export function debCategory(catalog: CategoryCatalog | null, applicationId: string | null | undefined): string {
  if (!applicationId) return UNKNOWN_CATEGORY;
  const assignments = catalog?.assignments.applications ?? FALLBACK_DEB_BY_ID;
  return categoryLabel(catalog, assignments[applicationId]);
}

export function devToolCategory(catalog: CategoryCatalog | null, toolId: string): string {
  const assignments = catalog?.assignments.developmentTools ?? FALLBACK_DEV_TOOL_BY_ID;
  return categoryLabel(catalog, assignments[toolId]);
}

// 给定当前出现的分类 label 集合，按 feed/内置顺序返回分类列表；未知分类兜底到最后。
export function orderedCategories(catalog: CategoryCatalog | null, present: ReadonlySet<string>): string[] {
  const categories = catalog?.categories ?? FALLBACK_CATEGORIES;
  const order = categories.map((category) => category.label);
  const known = order.filter((label) => present.has(label));
  const hasUnknown = [...present].some((label) => !order.includes(label));
  return hasUnknown ? [...known, UNKNOWN_CATEGORY] : known;
}
