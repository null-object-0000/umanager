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
2. **CI 频次提到 30 分钟**：`cron: "17,47 * * * *"`；
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

## 5. 效果

| 项 | 优化前 | 优化后 |
|---|---|---|
| 每次运行厂商下载量 | ≈ 4.5 GB | ≈ 0.3 GB |
| 每天厂商下载量（30 分钟频次） | ≈ 216 GB | ≈ 15 GB |
| 每天厂商下载量（原 6 小时频次） | ≈ 18 GB | ≈ 1.2 GB |
| 最坏检测延迟 | ≈ 9 h + 客户端 30 min | ≈ 30 min（+ 调度漂移）+ 客户端 15 min |

剩余项：**腾讯文档**是唯一仍每次整包下载的应用（~300 MB）：它的下载地址是固定
`…?version_id=latest`、官网版本由 JS 渲染、又因为 changelog 发布时间被当作权威版本时间
而失去可比较的 `Last-Modified`。补一个可用的 `versionField` / `pageVersionMarker`（或让
生成器把「changelog 时间未变」当作版本信号）即可消除这最后十几 GB/天。

## 6. 风险与已知取舍

- **`schedule` 不准时**：实测名义 6 小时对应的实际间隔是 median 6.28 h / max 8.91 h，说明
  GitHub 调度器会整体漂移。30 分钟的名义频次同样会漂移（可能被推迟或偶尔合并），所以
  收益是「每天次数变多、平均延迟下降」，不是「精确每 30 分钟」；
- **长尾运行 + `cancel-in-progress`**：`concurrency` 组内新运行会取消进行中的运行。去掉
  整包下载后单次运行应降到 1–3 分钟，30 分钟间隔留有充足余量；若某天某源又开始整包
  下载（探测失败 → 回退下载），最长运行时间会回来，需观察一次 CI 日志里的
  「重新下载整包」与「跳过 N 个」统计；
- **探测失败即回退**：CDN 不答 HEAD、网关不支持 HEAD、`Content-Length` 缺失等情况都会
  退回完整下载 —— 保守但会吃掉收益，这也是 `logReuseRefusal()` 存在的意义；
- **首次滚动**：`feed-sources.json` 新增的 `versionField`（飞书）要等第一次新 feed 发布后
  才成为可比证据，所以配置上线后的第一次运行仍会整包下载飞书。

## 7. 安全不变量影响

- **不改** `vendors.json`（编译进 App/helper 的事实源），本次改动不触发发版；
- feed schema 仍为 **v2**，没有新增/删除字段，只是「哪些条目需要重新抓」变了；
- 下载域名白名单、HTTPS、`size + SHA-256 + .deb 元数据` 三重校验、不可变计划、helper 验签
  全部不变；
- 复用只发生在「厂商证据表明未变更」时；证据不足即回到原来的完整下载路径（保守降级）。

## 8. 回滚

- 只想退回频次：把 `cron` 改回 `17 */6 * * *`（客户端 TTL 可保留，只是更频繁地命中
  同一版 feed）；
- 想完全关掉复用：在 `scripts/feed-download-reuse.mjs` 的 `decideDownloadSkip()` 顶部
  直接 `return { skip: false, reason: "disabled" }`；
- 单个源出问题：给它加 `forceDownload: true`（`feed-sources.json`，无需发版）。
