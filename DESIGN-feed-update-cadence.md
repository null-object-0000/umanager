# 设计：feed 更新频次与「未变更就不重复下载」

状态：已实现（2026-09）
相关文件：`.github/workflows/update-feed.yml`、`scripts/update-feed.mjs`、`scripts/feed-download-reuse.mjs`、`src-tauri/src/feed.rs`

## 1. 背景与问题

feed 的更新频次决定「厂商发新版 → 用户在 UManager 里看到」的最坏延迟。历史配置是
`cron: "17 */6 * * *"`（每天 4 次，注释写着 "Every 6 hours is more than enough"）。

实测现状（2026-09，最近 50 次调度 + 作业明细）：

| 指标 | 实测值 |
|---|---|
| 端到端耗时 | median ≈ 5.5 min（常见 2.5–7.5 min） |
| `generate (tencent)` | 79–206 s |
| `generate (common)` | 182–269 s |
| `merge`（含 Pages 部署） | 100–190 s |
| 单次异常 | 最长 1506 s（25 min，腾讯整包重试） |
| 实际调度间隔 | median 6.28 h、p90 8.16 h、max 8.91 h |

两点结论：

1. 名义 6 小时，但 GitHub 的 `schedule` 会成片漂移（我们的运行落在 UTC 04:5x / 11:2x /
   16:3x / 20:5x），所以「厂商发版 → feed 更新」最坏接近 **9 小时**；
2. 单次运行只占用 runner 约 6 分钟 / 6 小时（≈1.7%），**耗时不是提频的瓶颈**。

## 2. 真正的瓶颈：整包重复下载

`versionEndpoint` / `stableDownloadEndpoint` 与带 `.deb` 的 `releaseApi` 必须把整个 `.deb`
下载下来才能读到控制区 `Version` 并计算 SHA-256。按当时 feed 里的清单统计，**每次运行
重复下载 ≈ 4.5 GB**：

| 来源类型 | 每次运行下载量 | 代表应用 |
|---|---|---|
| `versionEndpoint` | 2854 MB | WPS 545 MB、Docker Desktop 440 MB、Trae 436 MB、飞书 338 MB、腾讯会议 188 MB、QQ 177 MB … |
| `stableDownloadEndpoint` | 1151 MB | 腾讯文档 302 MB、XMind 226 MB、微信 216 MB、Cursor 198 MB … |
| `releaseApi` | 469 MB | 思源 190 MB、洛雪 104 MB、Clash Verge 79 MB … |
| `aptRepository` | 0（用索引里的 SHA256） | VS Code、Chrome、Edge、微信输入法… |

按 6 小时频次约 18 GB/天；若直接提到 30 分钟就是 **≈216 GB/天** 打到厂商 CDN —— 既容易
触发厂商 WAF/限流，也不礼貌。所以提频必须先把重复下载去掉。

## 3. 决策

1. **先做「未变更就不重复下载」**（`scripts/feed-download-reuse.mjs`，纯函数 + 单测）；
2. **CI 频次名义提到 30 分钟**：`cron: "17,47 * * * *"`（实测 GitHub 只交付 4–5 次/天，见 §5）；
3. **客户端对齐**：`FEED_TTL` 15 → **10 分钟**，`FEED_REFRESH_INTERVAL` 30 → **15 分钟**，
   否则服务端提频会被客户端 30 分钟的轮询吃掉。

客户端带宽：一次完整刷新要拉 v3 中央 feed（463 KB）+ 两个源 feed（18 KB + 148 KB）≈ 630 KB，
15 分钟一次 ≈ 60 MB/天/客户端。GitHub Pages 的 100 GB/月软限制对应约 1 600 个客户端；用户量
上台阶时要重新评估（可选手段：把 `catalogJson` 从中央 feed 里再瘦身，或对源 feed 也用
`If-None-Match`）。

## 4. 复用判定规则（证据等级）

桌面 App 会用签名 feed 里的 SHA-256 校验下载到的 `.deb`，所以**复用陈旧条目 = 下载校验
失败**。因此每条规则都要求「厂商自己给出的、能证明字节未变」的证据；**任何信号缺失一律
照旧下载**。

| # | 适用来源 | 证据 | 说明 |
|---|---|---|---|
| 1 | `releaseApi` | Releases API 自带的 `asset.digest` 与上一版 `sha256` 相同 | 字节相同是**证明**（也等价于控制区版本相同），不需要探测 |
| 2 | 全部 | 下载 URL 完全相同 + 厂商版本字段未变 | `versionField` / `pageVersionMarker` 解析出的版本 |
| 3 | 全部 | 下载 URL 完全相同 + 源声明 `immutableDownloadUrl: true` | URL 内嵌版本/构建号，不会原地覆盖 |
| 4 | 全部 | 下载 URL 完全相同 + CDN `Last-Modified` 与上一版记录的 `versionUpdatedAt*`（source=`serverModified`）相同 | CDN 自己说「未修改」 |
| 5 | 轮换签名 URL 的源 | 源声明 `dynamicDownloadUrl: true`（或 `resolveAtDownload: true`）+ 厂商版本字段未变 | 飞书：URL 每次带新的 `x-signature`，只能靠版本字段 |

规则 2–5 一律额外要求**探测到的 `Content-Length` 等于上一版 `size`**，用于兜住「URL 不变、
版本号不变、但厂商原地重打包」的情形。另有 `forceDownload: true` 作为源级逃生开关。

探测实现：一次 `HEAD`（跟随重定向）；若 CDN 不答 `HEAD` 或没有 `Content-Length`（如腾讯
文档），退化为 `Range: bytes=0-0` 的 1 字节 GET，从 `Content-Range` 读总长，并立即丢弃
响应体。直连失败时同样走 `gatewayUrl` 网关。**探测只增加每应用一次 HEAD 的成本。**

### 源级 opt-in 一览

| 源 | 配置 | 依据 |
|---|---|---|
| obsidian | `immutableDownloadUrl` | `…/releases/download/v1.13.7/obsidian_1.13.7_amd64.deb` |
| trae | `immutableDownloadUrl` | `…/releases/stable/2.3.83560/linux/…` |
| qq-music | `immutableDownloadUrl` | 文件名含版本；若 `sign=` 参数轮换则自动退化为下载 |
| feishu | `versionField: data.version_number` + `resolveAtDownload` | 动态签名 URL，只能靠厂商自报版本 |

这些标记都是 **CI-only**：`toCatalogApplication()` 在写入签名 `catalogJson` 前会把
`immutableDownloadUrl` / `dynamicDownloadUrl` / `forceDownload` 连同 `gatewayUrl` /
`endpointHeaders` 一起剥掉，App 与 helper 看到的目录结构与之前完全一致（schema 仍是 v2）。

## 5. 实测：GitHub `schedule` 交付不了 30 分钟

**结论：`cron: "17,47 * * * *"` 被正确注册，但 GitHub 的调度器会把它合并成每天 4–5 次，达不到
30 分钟。要真正的 30 分钟必须用外部准点触发（见 §5.3）。**

### 5.1 历史（旧 6h cron，共 75 次 schedule 运行 / 21 天）

| 指标 | 实测 |
|---|---|
| 频次 | 75 次 / 21 天 ≈ **3.6 次/天**（名义 4 次/天） |
| 相对前一个名义槽位（00/06/12/18:17）的延迟 | 中位 **4.34 h**，最小 0.45 h，最大 5.71 h |
| 相邻间隔 | 最小 4.20 h、中位 6.64 h、最大 13.79 h |
| 延迟是否恶化 | 否（前 1/2 均值 3.64 h vs 后 1/2 均值 3.84 h） |

即：21 天里**每一次定时运行都迟到，最准的一次也晚了 27 分钟**。事件触发路径不受影响——tag
在 06:16:26 推送，`release` 运行同一秒创建。

### 5.2 新 cron（30 分钟）的实测

- **改 cron 后有数小时的注册过渡期**：06:16 推 `17,47`、07:16 推 `*/5` 探针、07:42 回滚，
  期间（含 disable/enable 重新注册）**5.5 小时零触发**；`*/5` 探针的 5 个槽位也全部没有触发。
- **注册稳定后能准点**：12:47 槽位在 **12:53:50 触发（+6 分钟）**。
- **但槽位会被合并/丢弃**：12:53:50 之后到 17:39:50 之间的 **9 个槽位（13:17 … 17:17）全部没跑**，
  两次实际触发间隔 **4.77 小时**。也就是说稳态频率仍 ≈ 4–5 次/天，与 6 小时 cron 无本质差别。

> 所以「每 30 分钟」目前只是**名义值**；`update-feed` 实际是「每天几次、时间点不保证」。
> 本轮改动真正的收益是**每次运行的厂商下载量从 ~4.5 GB 降到 ~0.3 GB**（见 §6），而不是频次。

### 5.3 要真正的 30 分钟：外部准点触发（待实施）

给已有的 Cloudflare Worker（`umanager.nichangen.workers.dev`，源码不在本仓库）加一个 Cron
Trigger，定时调 GitHub 的 dispatch API；Cloudflare cron 准点在分钟级，走的就是已经验证过的
`workflow_dispatch` 路径。GitHub 自带的 `schedule` 建议保留作兜底。

```js
export default {
  async scheduled(_event, env, ctx) { ctx.waitUntil(dispatchUpdateFeed(env)); },
  // …保留现有 fetch（/fetch 网关）路由…
};

async function dispatchUpdateFeed(env) {
  const res = await fetch(
    `https://api.github.com/repos/${env.REPO}/actions/workflows/${env.WORKFLOW}/dispatches`,
    {
      method: "POST",
      headers: {
        Authorization: `Bearer ${env.GITHUB_TOKEN}`,
        Accept: "application/vnd.github+json",
        "User-Agent": "umanager-feed-cron",
      },
      body: JSON.stringify({ ref: env.REF ?? "main" }),
    },
  );
  const ok = res.status === 204;
  console.log(ok ? "update-feed dispatched" : `dispatch failed: ${res.status} ${await res.text()}`);
  return ok;
}
```

```toml
# wrangler.toml
[triggers]
crons = ["17,47 * * * *"]

[vars]
REPO = "null-object-0000/umanager"
WORKFLOW = "update-feed.yml"
REF = "main"
```

需要两个 secrets：`GITHUB_TOKEN`（细粒度 PAT，仅本仓库、`Actions: write`）与 `DISPATCH_TOKEN`
（手动触发用）。注意：这**不是**把 feed 生成搬到 Worker 上——签名私钥仍只在 GitHub Actions
secret 里（安全不变量 8 不变），Worker 只负责准点按门铃。

### 5.4 顺带验证到的容错行为

17:39 那次运行里 `baidunetdisk` / `hexhub` / `dida` 三个源因 runner 侧 `fetch failed`（CN 端点
抖动）失败：探测拿不到大小 → 回退整包下载 → 下载也失败 → 复用上一版条目。线上源 feed 里这三个
应用仍在（`8.7.0` / `5.1.9` / `8.0.10`，共 25 个应用），即「绝不静默丢应用」的兜底链路工作正常。
同一次运行里腾讯文档是 `size-changed`（**真的**有新版本），说明复用规则没有盲目复用。

## 6. 效果

| 项 | 优化前 | 优化后 |
|---|---|---|
| 每次运行厂商下载量 | ≈ 4.5 GB | ≈ 0.3 GB |
| 每天厂商下载量（维持 4–5 次/天） | ≈ 18~22 GB | ≈ 1.2~1.5 GB |
| 每天厂商下载量（若 30 分钟真能落地） | ≈ 216 GB | ≈ 15 GB |
| 检测延迟 | 4–9 h（实测）+ 客户端 30 min | 4–5 h（GitHub 实际频次，见 §5）+ 客户端 15 min；若上外部触发则 ≈ 30 min + 15 min |

实测单次运行：`generate (common)` 24–306 s、`generate (tencent)` 75–89 s、`merge` 98–332 s，
端到端 **3–4.5 分钟**（复用之前是 5.5–8.6 分钟）。

剩余项：**腾讯文档**是唯一仍每次整包下载的应用（~300 MB）：它的下载地址是固定
`…?version_id=latest`、官网版本由 JS 渲染、又因为 changelog 发布时间被当作权威版本时间
而失去可比较的 `Last-Modified`。补一个可用的 `versionField` / `pageVersionMarker`（或让
生成器把「changelog 时间未变」当作版本信号）即可消除这最后十几 GB/天。

## 7. 风险与已知取舍

- **`schedule` 会迟到、会漏槽位**（§5 有完整实测）：名义 30 分钟的实际交付是每天 4–5 次。
  所以文档、UI 文案都不要承诺「30 分钟更新一次」；要承诺就得先上 §5.3 的外部触发；
- **改 cron 有注册过渡期**：改动 workflow 文件后（哪怕只是回滚 cron）本次观测到 5.5 小时
  无触发，排查「定时没跑」时要先把过渡期算进去，别急着判定调度器坏了；
- **长尾运行 + `cancel-in-progress`**：`concurrency` 组内新运行会取消进行中的运行。复用之后
  单次运行 3–4.5 分钟，30 分钟间隔余量充足；但若某源探测失败回退整包下载，运行时间会回到
  分钟级偏上，需看 CI 日志里的「重新下载整包」与「跳过 N 个」统计；
- **探测失败即回退**：CDN 不答 HEAD、网关不支持 HEAD、`Content-Length` 缺失等情况都会
  退回完整下载 —— 保守但会吃掉收益，这也是 `logReuseRefusal()` 存在的意义；
- **首次滚动**：`feed-sources.json` 新增的 `versionField`（飞书）要等第一次新 feed 发布后
  才成为可比证据，所以配置上线后的第一次运行仍会整包下载飞书。

## 8. 安全不变量影响

- **不改** `vendors.json`（编译进 App/helper 的事实源），本次改动不触发发版；
- feed schema 仍为 **v2**，没有新增/删除字段，只是「哪些条目需要重新抓」变了；
- 下载域名白名单、HTTPS、`size + SHA-256 + .deb 元数据` 三重校验、不可变计划、helper 验签
  全部不变；
- 复用只发生在「厂商证据表明未变更」时；证据不足即回到原来的完整下载路径（保守降级）。

## 9. 回滚

- 只想退回频次：把 `cron` 改回 `17 */6 * * *`（客户端 TTL 可保留，只是更频繁地命中
  同一版 feed）；
- 想完全关掉复用：在 `scripts/feed-download-reuse.mjs` 的 `decideDownloadSkip()` 顶部
  直接 `return { skip: false, reason: "disabled" }`；
- 单个源出问题：给它加 `forceDownload: true`（`feed-sources.json`，无需发版）。
