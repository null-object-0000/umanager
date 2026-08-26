# DESIGN：版本更新时间（`versionUpdatedAtUnixSeconds` + 来源标记）

> 状态：**最终方案，待实现。本文档不包含代码改动，只做实现依据。**
> 目标：让每个受管软件（含 UManager 自更新、开发工具）在签名 feed 里带上「当前版本的更新时间」。优先官方数据；拿不到官方数据时用「采集推断」：首次采集到某版本 → 无更新时间；之后某次采集发现版本升级 → 以该次采集时间作为版本更新时间。

## 0. 三个已确认的范围决定

| # | 决定 | 说明 |
|---|---|---|
| 1 | **支持开发工具（npm）** | `FeedToolEntry` 也带上版本更新时间，来源为 npm registry `time[version]`（官方数据） |
| 2 | **要有来源标记** | 新增 `versionUpdatedAtSource`，区分 `official` / `serverModified` / `observed` |
| 3 | **纳入 Last-Modified** | `aptRepository` / `stableDownloadEndpoint` 用 `.deb` 下载地址的 HTTP `Last-Modified` 作为「服务器修改时间」 |

## 1. 语义与字段

feed 中每个应用条目（`FeedApplicationEntry`）与开发工具条目（`FeedToolEntry`）新增两个**成对出现、可选**的字段：

```jsonc
{
  "version": "3.x.y",
  // ...
  "versionUpdatedAtUnixSeconds": 1750000000,   // 整数 unix 秒；null = 未知
  "versionUpdatedAtSource": "official"          // null = 未知
}
```

`versionUpdatedAtSource` 取值：

| 值 | 含义 | UI 文案建议 |
|---|---|---|
| `official` | 官方发布的精确时间（GitHub `published_at` / npm `time` / 端点时间字段） | 「官方发布时间」 |
| `serverModified` | 官方 `.deb` 文件的 HTTP `Last-Modified`（服务器文件修改时间，近似但可能受重打包影响） | 「官方安装包更新时间」 |
| `observed` | 采集推断：我们首次发现该版本（检测到升级）的采集时间 | 「首次采集到该版本」 |

**一致性不变量**：两个字段要么同时为 `null`，要么同时有值（`time` 有值且 `source` 有值）。校验时强制。

## 2. 数据流与安全不变量（不破坏任何现有不变量）

- 计算只发生在 CI（`scripts/update-feed.mjs`），结果随 **Ed25519 签名 feed** 下发；App 只读字段 + 展示，**不重新引入任何本机抓取**。
- `catalogJson` 的签名与授权链路不变：新字段只进 `feed.applications[id]` / `feed.developmentTools[id]` / `feed.selfUpdate`，不进 `catalogJson` 的签名正文（`catalogJson` 仍是 source 描述符数组）。
- feed `schemaVersion` **保持 2**：新字段可选、加性。老 App 忽略未知字段；新 App 用 `#[serde(default)]` 兼容旧 feed。
- **helper 零改动、plan schema v2 不变。**

## 3. 状态来源：用「上一次发布的 feed.json」当参照物

CI 是无状态的。持久化参照物直接用**上一次发布到 Pages 的 feed**：

- 生成器开头 `fetchPreviousFeed(catalog.metadataFeed?.url)`（best-effort）：
  - 成功 → `previous`；
  - 失败（首次部署 / 404 / 网络）→ `previous = null`，只告警不中断，走「无历史」分支。
- 可选：支持环境变量 `PREVIOUS_FEED_URL`（或本地 `PREVIOUS_FEED_PATH`）覆盖，便于本地 `npm run update-feed` 联调。

> 兼容性：旧 feed（本特性上线前）里没有新字段，`previous.version` 仍然存在，比较版本即可正确工作；旧值默认 `null` → 版本未变继续 `null`，版本变了记本次时间。

## 4. 官方时间候选（按 source kind）

| kind | 候选来源 | `source` 值 | 获取方式 |
|---|---|---|---|
| `releaseApi` | GitHub `published_at` → 兜底 `created_at` | `official` | `releaseApiEntry` 内 `json.published_at ?? json.created_at` |
| `versionEndpoint`（JSON 且配置 `releaseTimeField`） | 端点时间字段 | `official` | `getJsonPath(payload, source.releaseTimeField)` |
| `versionEndpoint`（HTML 或无 `releaseTimeField`） | 无 | — | 回退采集推断 |
| `aptRepository` | `.deb` 下载地址 `Last-Modified` | `serverModified` | 对 `entry.downloadUrl` 发一次 `HEAD`，读 `last-modified` 头 |
| `stableDownloadEndpoint` | `.deb` 下载地址 `Last-Modified` | `serverModified` | 同上（HEAD，best-effort，无网关回退） |
| `developmentTools`（npm） | registry `time[version]` | `official` | `toolEntry` 改拉全量文档 `registry.npmjs.org/<pkg>` |

> Last-Modified 用「直接 HEAD、best-effort」实现：失败/无头/非 2xx → `null`，自然回退采集推断。不做网关代理回退（网关是 GET 转发，不保证透传 `last-modified`，收益低）。

## 5. 采集推断算法（核心，含来源标记）

抽出纯函数模块 `scripts/version-time.mjs`，供 `update-feed.mjs` 调用、供单测覆盖：

```js
// 解析：ISO 8601 字符串或纯数字 → 整数秒；失败返回 null
export function parseUnixSeconds(value) { /* ... */ }

// HTTP Last-Modified "Wed, 21 Oct 2015 07:28:00 GMT" → 整数秒；失败 null
export function parseLastModified(headerValue) { /* ... */ }

// previous: { version, versionUpdatedAtUnixSeconds, versionUpdatedAtSource } | null | undefined
// version:   本次抓到的当前版本（Debian 控制字段权威版本）
// candidate: { time, source } | null（本次的 official / serverModified 候选）
// now:       本次采集的 unix 秒
export function mergeVersionUpdatedAt(previous, version, candidate, now) {
  if (!candidate) {
    if (!previous) return null;                                   // 首次采集：无基线
    if (previous.version !== version) return { time: now, source: "observed" }; // 检测到升级
    return (previous.versionUpdatedAtUnixSeconds != null && previous.versionUpdatedAtSource)
      ? { time: previous.versionUpdatedAtUnixSeconds, source: previous.versionUpdatedAtSource }
      : null;
  }
  if (candidate.source === "official") return candidate;          // 官方时间权威，总是采用（含回填基线）
  // serverModified：版本变化/无历史/此前为 observed 时采用；否则沿用旧值，防重打包导致时间抖动
  if (!previous || previous.version !== version) return candidate;
  if (!previous.versionUpdatedAtUnixSeconds || previous.versionUpdatedAtSource === "observed") return candidate;
  return { time: previous.versionUpdatedAtUnixSeconds, source: previous.versionUpdatedAtSource };
}
```

要点：

- **官方候选优先于采集推断**；`official` 权威、稳定，总是采用（因此后续拿到官方数据时能自动回填未知基线）。
- `serverModified` 只在「版本变化 / 无历史 / 此前是 observed」时写入，避免 `.deb` 被 CDN 重打包（版本不变但 mtime 变）导致时间反复跳动。
- 版本比较用 **`!==` 精确比较**（任何版本字符串变化都算升级），不调 `compareVersions`。
- 采集推断的「observed」时间 = 检测到变化的那次采集时间（即 `now`）。

## 6. 分文件改动清单

### 6.1 `scripts/version-time.mjs`（新增，CI-only）

- 导出 `parseUnixSeconds` / `parseLastModified` / `mergeVersionUpdatedAt`。

### 6.2 `scripts/version-time.test.mjs`（新增，CI-only）

- 覆盖：首次采集 → null；升级 → observed=now；未变且无历史值 → null；未变且有值 → 沿用；official 优先与回填；serverModified 的三条采用规则与「不抖动」规则；时间解析成功/失败。

### 6.3 `scripts/update-feed.mjs`（CI-only，不发版）

- 引入 `version-time.mjs` 的辅助函数。
- 新增 `fetchPreviousFeed(url)`：拉上一版 feed，任何失败返回 `null` 并 `log` 告警。
- 新增 `lastModifiedOf(url)`：`HEAD` 请求读 `last-modified`，best-effort。
- `releaseApiEntry`：解析 `json.published_at ?? json.created_at` → `official` 候选。
- `versionEndpointEntry`：JSON 模式下若 `source.releaseTimeField` 存在，`getJsonPath` 取时间 → `official` 候选。
- `aptEntry` / `stableDownloadEntry`：`lastModifiedOf(downloadUrl)` → `serverModified` 候选。
- `toolEntry`：改拉 `registry.npmjs.org/<pkg>` 全量文档，`version = doc["dist-tags"].latest`，`official` 候选 = `doc.time[version]`。
- `main()` 汇总处：
  - 对 `applications` / `selfUpdate` / `developmentTools` 分别用 `mergeVersionUpdatedAt(previous对应条目, entry.version, candidate, now)` 算出 `versionUpdatedAtUnixSeconds` + `versionUpdatedAtSource` 写回 entry。
  - 更新文件头注释的 Output schema。

### 6.4 `crates/umanager-catalog/src/lib.rs`（需发版）

- `SourceSpec::VersionEndpoint` 增加可选字段：

```rust
/// JSON 模式下：端点响应里表示该版本发布时间的点分路径（官方发布时间）。
#[serde(default)]
pub release_time_field: Option<String>,
```

（`resolve_download_url` 的解构已带 `..`，无需改；现有 `versionEndpoint` 反序列化测试不受影响，字段默认 `None`。）

### 6.5 `src-tauri/src/feed.rs`（需发版）

- 新增枚举：

```rust
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum VersionUpdatedAtSource { Official, ServerModified, Observed }
```

- `FeedApplicationEntry` 与 `FeedToolEntry` 各加：

```rust
#[serde(default)] pub version_updated_at_unix_seconds: Option<u64>,
#[serde(default)] pub version_updated_at_source: Option<VersionUpdatedAtSource>,
```

- `validate_application_entry`（及 dev tool 校验）加：两个字段必须同时有值或同时为 `null`；`Some(0)` 视为非法。

### 6.6 `src-tauri/src/source_engine.rs`（需发版）

- `ApplicationDetails` 加两个字段，`feed_details` 从 entry 透传。
- 内部 `Installable` 加两个字段，`feed_installable` 与「无 feed 条目」的早退分支透传。
- **更新两处测试里的 `FeedApplicationEntry { ... }` 字面量**，补 `version_updated_at_unix_seconds: None, version_updated_at_source: None`（否则新字段会让 struct 字面量编译失败）。
- 新增断言：`feed_details` 正确透传两个字段。

### 6.7 `src-tauri/src/installable.rs`（需发版）

- `InstallableApplication` 加两个字段，`offer_for` 从 `Installable` 透传。

### 6.8 前端 `src/types.ts`（需发版）

```ts
export type VersionUpdatedAtSource = "official" | "serverModified" | "observed";
```

- `ApplicationDetails`、`InstallableApplication` 各加 `versionUpdatedAtUnixSeconds: number | null; versionUpdatedAtSource: VersionUpdatedAtSource | null;`

### 6.9 前端 `src/App.tsx`（需发版）

- 新增一个小工具函数（复用现有 `formatTime` 风格，但**含年份**，例如 `toLocaleString("zh-CN", { year, month, day, hour, minute })`）。
- 按 `source` 显示标签（见 §1 表格）。
- 展示位置：
  - 详情抽屉「版本」区：加一行「版本更新时间」；
  - 安装抽屉「新安装目标」区：加一行；
  - 自更新抽屉「最新版本」facts：加一行（`getSelfUpdateStatus` 复用 `ApplicationDetails`，自动生效）；
  - （可选，后置）商店/开发工具行在候选版本旁加小字。

### 6.10 不动的部分

- `.github/workflows/update-feed.yml`：无需改（拉上一版 feed 是普通出站 HTTPS；Pages 已具备网络）。
- `crates/umanager-helper`、`crates/umanager-plan`、`operation_plan.rs`：零改动。

## 7. 边界情况

| 场景 | 结果 |
|---|---|
| 首次部署 / Pages 未就绪 / 旧 feed 拉取失败 | 走「无历史」→ 官方候选优先，否则 `null` |
| 新增一个软件 | 首次采集无基线 → 官方候选优先，否则 `null` |
| 版本变化（含回退） | `observed = now`（无官方候选时） |
| 官方时间本次拿不到、下次才拿到 | `official` 候选在拿到那次即回填（即使此前是 `null` 基线或 `observed`） |
| 旧 App 读新 feed | 忽略新字段，完全兼容 |
| 新 App 读旧 feed | 字段缺失 → `null` → UI 显示「—」 |
| 版本不变但 `.deb` 被重打包（mtime 变） | `serverModified` 不覆盖已有 `official`/`serverModified`，不抖动 |

## 8. 测试计划

1. **JS 单测**：`npm test` 跑 `scripts/version-time.test.mjs`（vitest 默认会拾取 `*.test.mjs`；若未拾取则在 `vitest` 配置里显式 include）。
2. **catalog crate**：`cargo test --manifest-path crates/umanager-catalog/Cargo.toml` —— 新增 `releaseTimeField` 可选反序列化断言。
3. **主程序**：`cargo check` + `cargo test --manifest-path src-tauri/Cargo.toml` —— 更新 `source_engine` 的两处 struct 字面量，新增字段透传断言。
4. **前端**：`npm run build`（tsc 会校验新类型）；`npm test` 全绿。
5. **生成器**：本地 `npm run update-feed`（需网络）检查 `dist/feed.json` 里各条目带 `versionUpdatedAtUnixSeconds` / `versionUpdatedAtSource`，且 `releaseApi` / npm 工具为 `official`，apt/stable 为 `serverModified` 或 `observed`/`null`。
6. **端到端**：发版后，详情/安装/自更新抽屉展示「版本更新时间」与来源标签；老版本 App 不受影响。

## 9. 兼容性与发布

- **无需 bump feed schemaVersion**（保持 2），但主程序、helper、生成器三处如果将来任何一处升级 schema，仍须按 AGENTS.md 同步。
- 本次改动涉及 App（Rust + 前端），**需要发版**；`feed-sources.json` 里给 `versionEndpoint` 追加的 `releaseTimeField` 属 CI-only 配置，随签名 feed 下发、免单独发版。

## 10. 开放问题 / 待定细节

- [ ] `observed` / `serverModified` / `official` 的最终 UI 文案（§1 已给建议，实现时可微调）。
- [ ] 商店/开发工具**列表行**是否展示更新时间（建议后置，先落详情抽屉）。
- [ ] `Last-Modified` 是否值得在个别厂商（HEAD 被墙/403）配置关闭项（当前方案：统一 best-effort，不加开关）。
