# DESIGN：UManager 多软件源（中央源 + 可发现源 + 验签链）

> 状态：**方向已确认，分期实现。** 本文档是 app 侧多源的实现依据；CI 双源拆分（第 7 节）作为第一期先行落地。
> 已确认的三个范围决定：

| # | 决定 | 说明 |
|---|---|---|
| 1 | **信任模型 B：完整第三方源（验签链）** | 每个源有独立私钥，中央源注册表背书其公钥；安装第三方源新增软件时，计划里携带「中央验签链 + 源自身的 `catalogJson`+签名」，特权 helper 验链后授权 |
| 2 | **feed schema 升 v3、plan schema 升 v3** | feed 与 plan 同步升级；主程序、helper、`scripts/update-feed.mjs` 三处同步修改（AGENTS.md 约束 #9） |
| 3 | **分期：CI 双源拆分先落地** | 第一期只改 CI（生成/发布层），中央 feed 输出合同保持 v2 不变，旧版 App 无感；app 侧多源（feed.rs / UI / helper 验链）第二期做 |
| 4 | **路径版本化：v3 世界整体放 `/v3/`，0.9.1 起为准** | `/v3/feed.json`（瘦中央）+ `/v3/feed.<id>.json`（各源）独立发布、独立密钥；0.9.1 起 `metadataFeed.url` 指向 `/v3/feed.json`；`/feed.json`（v2 聚合）双轨保留服务 ≤0.9.0，弃用后移除。无需 schemaVersion 断代翻转 |

## 1. 概念

- **中央源（central source）**：umanager 自己运营、内置公钥验证的源，即现在发布在
  `https://null-object-0000.github.io/umanager/feed.json` 的 feed。它是：
  - **信任锚**：App 和 helper 内置中央公钥（hex `57d369d3…0c8f9`，见 `src-tauri/src/feed.rs`
    `FEED_PUBLIC_KEY_HEX` 与 `crates/umanager-helper/src/main.rs`）；
  - **注册表**：在新增的 `sources[]` 里背书其他源的 url / hosts / 公钥；
  - **umanager 自身的 feed**：`selfUpdate` 字段即「umanager 软件本身的版本信息」，保持只在中央源发布。
- **源（source / feed source）**：一个独立签名、独立周期的 feed（`feed.<id>.json` + `.sig`），
  由中央源注册表背书才能被 App 接受。示例：`tencent`（腾讯源）、`common`（公共源），
  未来可加 `bytedance` / `microsoft` / 第三方社区源。
- **分组（CI 概念）**：生成侧的「一次抓取一个源」= 未来的「一个源 feed」。每个应用归属一个
  `sourceGroup`；CI 按组并行抓取。分组与源一一对应，后续把某组发布成独立 feed 不需要再改应用归属。

## 2. 动机与收益

1. **生成侧并行（第一期已可兑现）**：现在所有 19 个应用 + selfUpdate + 6 工具在一个 job 里**串行**抓取，
   每个多余应用的 `.deb` 还下载两遍（条目一次 + 图标一次），单次 `update-feed` 约 6.5 分钟。
   按源拆分后：组内并发抓取 + `.deb` 只下载一次 + 图标不变跳过下载，关键路径降到 ~2-3 分钟；
   且腾讯源（中国端点 + 网关 + 大包）挂掉时，公共源照常发布，其余应用回落上一版条目。
2. **消费侧多源（第二期）**：
   - 中央源可发现其他源 → App「软件源」页展示，用户可开关、单独刷新、看每个源的信任状态与上次刷新时间；
   - 某个源故障只影响该源（各自有缓存与验签状态），不再整体不可用；
   - 第三方源（社区镜像、公司内源）成为可能，无需发新版 App 即可接入新软件目录。
3. **umanager 自身 update feed 独立可见**：`selfUpdate` 固定在中央源，UI 里中央源一行即可
   「检查更新 + 看 releaseNotes」（现状行为不变，只是多了一个可点击的来源卡片）。

## 3. 现状盘点（要动的地方）

| 位置 | 现状 | v3 后 |
|---|---|---|
| `src-tauri/src/feed.rs` | `FEED_SCHEMA_VERSION=2`；`validate()` 对 `schemaVersion != 2` 直接报「不支持的元数据源版本」；单 feed 拉取 + 内置公钥验签；缓存 `feed/feed-cache.json` | 支持 v3 解析；按源拉取/验签/缓存；`sources[]` 校验（id/url/hosts/pubkey/字段格式） |
| `crates/umanager-catalog/src/lib.rs` | feed 结构体（v2 字段） | 加 `source` 自描述 + `sources[]` 注册表结构体 |
| `crates/umanager-helper/src/main.rs` | 只认内置公钥 + 计划内 `catalogJson`/`catalogSignature` | 增加「中央背书源记录 → 源公钥 → 源 catalogJson」两级验签链 |
| `crates/umanager-plan/src/lib.rs` | `PLAN_SCHEMA_VERSION=2`；不可变计划 + 15 分钟有效性 | plan v3：新增源链字段（见 §5） |
| `src-tauri/src/operation_plan.rs` | `signed_catalog_auth()` 把中央 feed 的 catalogJson 对放进计划 | 按应用来源选「中央目录」或「源目录 + 背书链」 |
| `src-tauri/src/lib.rs` / 前端 | 设置页无来源概念 | 「软件源」页 + 各源状态/开关/刷新 |
| `scripts/update-feed.mjs` | 单 pass 生成单一 feed | `--group/--partial/--merge` 三模式（第一期）；第二期把每组输出成独立 `feed.<id>.json` |
| `.github/workflows/update-feed.yml` | 单 generate job | 矩阵 generate（每源一个）+ merge job（第一期落地） |

## 4. feed schema v3

### 4.1 中央源 feed（v3：瘦中央，`/v3/feed.json`）

> **路径版本化定案**：v3 世界整体发布在 `/v3/` 路径下（`/v3/feed.json` + `/v3/feed.<id>.json`），
> 与 v2 的 `/feed.json` 完全独立、双轨并存。0.9.1 起 App 的 `vendors.json` 里
> `metadataFeed.url` 直接指向 `/v3/feed.json`，无需任何 schemaVersion 断代翻转；
> `/feed.json` 继续由一期逻辑生成（v2 全量聚合），服务 ≤0.9.0 的旧 App，弃用后整体移除。

```jsonc
{
  "schemaVersion": 3,
  "source": {
    "id": "umanager",
    "name": "UManager 中央源",
    "role": "central",
    "url": "https://null-object-0000.github.io/umanager/v3/feed.json",
    "hosts": ["null-object-0000.github.io"],
    "publicKeyHex": "57d369d3…0c8f9"
  },
  "sources": [
    {
      "id": "tencent",
      "name": "腾讯源",
      "role": "vendor",
      "url": "https://null-object-0000.github.io/umanager/v3/feed.tencent.json",
      "hosts": ["null-object-0000.github.io"],
      "publicKeyHex": "<该源 feed 签名公钥 hex>",
      "defaultEnabled": true,
      "description": "腾讯系应用（QQ / QQ 音乐 / 腾讯文档 / CodeBuddy / 微信 / 腾讯会议）"
    },
    {
      "id": "common",
      "name": "公共源",
      "role": "common",
      "url": "https://null-object-0000.github.io/umanager/v3/feed.common.json",
      "hosts": ["null-object-0000.github.io"],
      "publicKeyHex": "<该源 feed 签名公钥 hex>",
      "defaultEnabled": true
    }
  ],
  "generatedAtUnixSeconds": 0,
  // 瘦中央：不再携带全量 applications —— 应用由各源 feed 提供，App 端逐源拉取后合并
  "selfUpdate": { /* umanager 自身的版本信息，仅中央源 */ },
  "developmentTools": { /* 全部工具 */ },
  "catalogJson": "<中央自有新增软件目录（当前为空；各源新增走源自己的 catalogJson + 验签链）>",
  "catalogSignature": "<Ed25519 签名，中央私钥；目录为空时为一对空串/缺省>",
  "categories": [ /* 不变 */ ],
  "categoryAssignments": { /* 不变 */ }
}
```

- **`source` 自描述 + `sources[]` 注册表是 v3 相对 v2 的全部新增**；`applications` 从中央源
  **移除/清空**（瘦中央），不再有冗余聚合——这正是多源的意义：每个源独立维护自己的应用，
  第三方源无需中央聚合即可上架。
- `selfUpdate` / `developmentTools` / `categories` 保持**仅中央源**发布（中央独有数据）。
- 中央的 `catalogJson`/`catalogSignature` 只覆盖**中央自有新增软件**（当前无）；各源新增软件
  由各源 feed 自己的目录 + 验签链授权（§5），**不再是「中央目录聚合各源」**。

### 4.2 源 feed（`/v3/feed.<id>.json`）

```jsonc
{
  "schemaVersion": 3,
  "source": {
    "id": "tencent",
    "name": "腾讯源",
    "role": "vendor",
    "url": "https://null-object-0000.github.io/umanager/v3/feed.tencent.json",
    "hosts": ["null-object-0000.github.io"],
    "publicKeyHex": "<该源自己的公钥>"
  },
  "generatedAtUnixSeconds": 0,
  "applications": { "qq": { /* 仅本源应用 */ }, "qq-music": { /* … */ } },
  "catalogJson": "<本源新增软件目录>",
  "catalogSignature": "<本源私钥签名>"
  // 没有 selfUpdate / sources / categories / developmentTools
}
```

- 源 feed 只声明**它自己的应用**与**它自己的目录**；工具、selfUpdate、分类聚合都只在中央源。

### 4.3 兼容与校验

- `feed.rs::validate()` 对 v3：校验 `source`/`sources[]` 字段格式（id 非空、url https、
  hosts 精确白名单语法复用 `host_matches` 语义、`publicKeyHex` 是 32 字节 hex）；
  对 `sources[]` 里的每个条目还要校验 id 不重复、id != 中央 id。
- 0.9.1 起 App 只从 `/v3/feed.json` 拉取（`metadataFeed.url`），**`validate()` 接受 v2 | v3**
  （容忍本地缓存里的旧格式、以及未来回滚场景）；v2 版中央 `/feed.json` 是另一条路径，
  由旧 App 使用，不存在「同一路径换了 schema 导致断代」的问题。
- helper 不需要解析整个 v3 feed，它只看到计划里携带的链字段（§5），格式同步校验。

## 5. 信任模型 B：验签链与 plan v3

### 5.1 两级信任

```
内置中央公钥（编译期）
   └─ 验签 → 中央 feed.json（含 sources[] 注册表）
                 └─ 背书 → { sourceId, url, hosts, publicKeyHex }   ← sourceRef
                                └─ 验签 → 源 feed.<id>.json
                                              └─ 携带 → { 源 catalogJson, sourceCatalogSignature }
                                                           └─ 验签通过 → 该源的新增软件可被 helper 授权
```

### 5.2 plan v3 新增字段（`crates/umanager-plan`）

对「安装/卸载目录来自**非中央源**」的应用，`InstallPlan`/`RemovePlan` v3 增加：

```jsonc
{
  "schemaVersion": 3,
  // …现有字段不变（planId=SHA-256(payload)、15 分钟有效、归属当前用户）…
  "sourceRef": {
    "sourceId": "tencent",
    "feedUrl": "https://null-object-0000.github.io/umanager/feed.tencent.json",
    "publicKeyHex": "<该源公钥 hex>"
  },
  "sourceEndorsement": "<中央私钥对上面 sourceRef 原文(UTF-8 JSON 按字节)的 Ed25519 签名 hex>",
  "sourceCatalogJson": "<源 feed 的 catalogJson 原文>",
  "sourceCatalogSignature": "<源私钥对 sourceCatalogJson 的 Ed25519 签名 hex>"
}
```

helper 校验顺序（新增步骤插在现有 `catalogJson` 校验之前/并列）：

1. 计划整体：SHA-256 payload = `planId`、有效性、归属（现有逻辑不变）；
2. 无 `sourceRef` → 走现有路径（内置公钥直接验 `catalogJson`/`catalogSignature`）——**中央源应用完全兼容 v2 路径**；
3. 有 `sourceRef`：
   a. 内置公钥验 `sourceEndorsement` 覆盖 `sourceRef` 原文（`sourceRef` 必须按字节原文编码，禁止重新序列化）；
   b. `sourceRef.sourceId` 与 `sourceRef.feedUrl`/`publicKeyHex` 非空、格式合法；
   c. `sourceRef.publicKeyHex` 验 `sourceCatalogSignature` 覆盖 `sourceCatalogJson` 原文；
   d. 目标应用 `applicationId` ∈ `sourceCatalogJson` 解析出的目录；
   e. 之后**所有现有复核不变**：包名/架构/是否可卸载、`.deb` 元数据与 SHA-256、hosts 白名单、本机安装状态 → 固定 `dpkg`。
4. helper 不新增任何「按源放行的长期白名单」——每个计划独立携带完整链，计划即授权（与现有
   「计划不可变 + 计划内已签名 catalogJson」哲学一致）。

### 5.3 威胁分析

| 场景 | 后果 | 缓解 |
|---|---|---|
| 源私钥泄露 | 攻击者能对该源发假版本/假目录 | 源只影响它自己的应用；helper 仍复核 `.deb` SHA-256 与 hosts；可通过中央注册表轮换/下线该源（§5.4） |
| 中央私钥泄露 | 全源可控 | 与现状等同（今日中央 feed 已能新增任意软件）；维持现状的密钥保管纪律（Actions secret） |
| 中间人篡改源 feed | 验签失败 | App 丢弃该源并标记「签名失败」，回落到该源上一版缓存；`sources[].hosts` 精确白名单限制可指向的下载域名 |
| 注册表被篡改（feed 被换） | 指向攻击者源 | 中央 feed 本身必须过内置公钥验签；`sources[].publicKeyHex` 变化时 App 提示「源密钥变更」（§6.3） |
| 重放旧源 feed | 版本回退 | 与现状一致：App 合并时优先更高版本；旧源 feed 的 `generatedAtUnixSeconds` 陈旧时仅作参考 |

### 5.4 密钥轮换与下线

- 换源密钥：中央注册表更新 `publicKeyHex` 即可（App 提示确认）；helper 每计划验链不受影响。
- 下线源：从 `sources[]` 移除 + 该源应用回落中央已有条目（它是聚合源，原本就有）；App 停拉该源，本地缓存过期后自然消失。
- 新源上架：中央注册表加条目 + 发布该源 feed；App 下一轮拉中央 feed 即发现，无需发版。

## 6. App 侧改动（第二期）

### 6.1 拉取与缓存（`src-tauri/src/feed.rs`）

- 中央源（0.9.1 起）：拉 `/v3/feed.json`+`.sig`，内置公钥验签，写入 `feed/feed-cache.json`。
  瘦中央不带 `applications`，只提供注册表 + 中央独有数据。
- 新状态：`AppFeedState` 增加 `sources: HashMap<sourceId, SourceState>`；
  `SourceState = { registry: SourceInfo(来自中央注册表), cache: Option<FeedCache>, lastFetch: …, status: verified | signatureFailed | fetchFailed }`。
- 源缓存文件：`feed/source-<id>.json` + `feed/source-<id>.json.sig`，与中央缓存同目录，
  每次读取重新验签（沿用现有 stale-while-revalidate 语义：先返回缓存、后台按源各自刷新）。
- 合并规则（v1，瘦中央）：`applications = { ...各启用的源合并... }`，按**注册表顺序**逐源叠加；
  同一 `applicationId` 出现在多个源时，**前面的源胜出**（中央不再有应用可覆盖源）。版本比较沿用现有
  `compare` 逻辑不做跨源「最高版本胜出」（避免第三方源用高版本号劫持——v1 保守）。

### 6.2 计划生成（`src-tauri/src/operation_plan.rs`）

- `signed_catalog_auth(app)`：查 `applicationId` 归属——中央目录里有 → 现有路径（中央 `catalogJson`+签名）；
  否则在**已启用的源**目录里找 → 组装 plan v3 的 `sourceRef/sourceEndorsement/sourceCatalogJson/sourceCatalogSignature`。
- `sourceEndorsement` 在 App 拉中央 feed 时缓存（central 对每个源记录的签名），不临时联网签名。

### 6.3 UI：设置页「软件源」

- 列表：中央源（固定、含「umanager 自身更新」入口：版本 + releaseNotes + 检查更新）+ 各已发现源卡片（名称/角色/地址/默认启用标记）。
- 每张卡片：信任状态（已验证 / 签名失败 / 拉取失败 / 密钥变更待确认）、上次刷新时间、开关、单独刷新。
- 「密钥变更」确认弹窗：注册表里某源 `publicKeyHex` 与上次不一致时，要求用户确认后才切换信任（防静默换钥）。

## 7. CI 改动（第一期，先行落地）

> 第一期的产出合同：**中央 feed 仍是 schemaVersion 2**，结构与今天的输出完全一致（旧版 App 无感）；
> 同时**每个源发布独立签名 feed**（`feed.tencent.json` / `feed.common.json` + 各自 `.sig`，暂用同一把私钥），
> 旧版 App 不读它们、新版 App 上线后直接可发现。二期（§4/§8）把 v3 世界整体发布到 `/v3/` 路径、
> 各源换独立密钥，第一期这套根路径产物继续作为 v2 兼容入口双轨保留。

### 7.1 源注册表（数据驱动）

- `feed-sources.json` 新增顶层 `sources`：`{ "<sourceId>": { "name": …, "role": … } }`。
- 每个额外应用新增可选 `sourceGroup`（缺省 `common`）。
- 内置应用（`vendors.json`）的归属在 `scripts/update-feed.mjs` 内置表里维护（wechat/wemeet → tencent），
  不动 app 编译产物。
- 脚本启动校验：所有 `sourceGroup` 必须 ∈ `sources` 注册表；未注册的组直接报错，防止应用被静默漏抓。

### 7.2 `scripts/update-feed.mjs` 三模式

| 模式 | 命令 | 行为 |
|---|---|---|
| 全量（默认，本地/兜底） | `node scripts/update-feed.mjs [OUT]` | 与今天一致：单 pass 抓全部 → 上一版兜底 → version-time → 图标 → `catalogJson`/feed 签名 → 写 feed（含 selfUpdate/工具/分类） |
| 生成源 feed | `--group <id> --out dist/feed.<id>.json` | 只抓本组应用（+本组额外应用的图标），**就地完成**上一版兜底与 version-time 合并，产出该组**最终签名源 feed**（v2 结构：`applications` + 本组 `catalogJson`/`catalogSignature`） |
| 合并发布 | `--merge --parts <dir> --out dist/feed.json` | 发现 `<dir>/<group>/feed.<group>.json` 全部源 feed；按 feed-sources.json 顺序聚合 applications/catalogJson，缺失组回落到上一版中央 feed；就地抓取中央独有数据（selfUpdate/开发工具）+ version-time；把各源 feed（+.sig）复制进发布目录，签名写 feed |

- 并发：组内应用抓取用有界并发（默认 5）；`.deb` 在条目阶段只下载一次，
  图标提取**复用**该文件；上一版目录已有图标且条目 sha256 未变时跳过下载直接沿用。
- 兜底：每组生成时就对**自己组**的应用做 `entryOrPrevious` 回落上一版中央 feed；merge 只对
  **整组失败/缺失**的应用做中央级回落——两处都不会静默丢应用。

### 7.3 `.github/workflows/update-feed.yml`

```text
generate（matrix: group = [tencent, common]，并行）
  └─ node scripts/update-feed.mjs --group ${{ matrix.group }} --out dist/feed.<group>.json
     （env: FEED_SIGNING_KEY + GITHUB_TOKEN → 产出已签名的独立源 feed）
  └─ upload artifact feed-<group>（dist/feed.<group>.json + .sig + dist/icons，if: always()）

merge（needs: generate；if: always()）
  ├─ download feed-tencent / feed-common → partials/<group>
  └─ node scripts/update-feed.mjs --merge --parts partials --out dist/feed.json
     （聚合 + 中央独有数据 + 复制源 feed 进 dist）
  └─ configure-pages / upload-pages-artifact ./dist / deploy-pages
     （Pages 上：feed.json + feed.tencent.json + feed.common.json，各自 .sig + icons/）
```

- 某个 generate job 失败 → 该组源 feed 缺失 → merge 对该组应用走「上一版中央 feed」兜底，其余源照常发布。
- 加新源 = 注册表加一行 + 个别应用标 `sourceGroup` + matrix 数组加一个 id；不改脚本逻辑。
- 第二期切换各源独立密钥时，只需 group job 改用该源私钥签名 + 中央 feed v3 注册表带各源公钥。

## 8. 发布/滚动步骤

1. **第一期（已完成）**：CI 双源拆分（§7）——产出仍是 v2 中央 `feed.json` + 根路径独立源 feed，
   App 不需要发版。收益：`update-feed` 从 ~6.5 分钟降到 ~2 分钟，腾讯源故障不再拖累全局。
2. **第二期（0.9.1 发版）**：feed schema v3 结构 + `crates/umanager-catalog` 结构体 + `feed.rs`
   读 `/v3/feed.json` 的注册表 → 逐源拉取/验签/合并（瘦中央，`validate()` 接受 v2 | v3）+ per-source
   缓存/状态 + 设置页「软件源」UI + plan v3 + helper 验签链，一起进入 0.9.1。
3. **v3 世界上线（与 0.9.1 同批）**：CI 开始发布 `/v3/feed.json`（瘦中央）与 `/v3/feed.tencent.json` /
   `/v3/feed.common.json`（各源），源签名私钥换成各源独立密钥（`FEED_SIGNING_KEY_TENCENT` 等），
   公钥写进注册表；0.9.1 的 `vendors.json` `metadataFeed.url` 指向 `/v3/feed.json`。
   **无需断代**：`/feed.json`（v2 聚合）继续由一期逻辑生成，服务 ≤0.9.0 旧 App；
   弃用旧 App 后从 CI 移除 `/feed.json` 发布即可。
4. **第三方源**：提供「源接入模板」（生成密钥、构建源 feed 的最小脚本 + 文档），
   helper 链能力复用，不开新机制。

## 9. 测试计划

- 第一期：
  - `scripts/feed-merge.test.mjs`：分组解析、源 feed 聚合顺序、上一版兜底、缺失组回退、catalog 顺序恒定、version-time；
  - 本地冒烟：`--group tencent` + `--group common` 生成源 feed + `--merge`，对比产物结构与当前 feed 关键字段；
  - CI 实测一次完整 `update-feed`，核对 `applications/extraCatalog/tools` 计数、独立源 feed 落盘与页面部署。
- 第二期：
  - `feed.rs`：v2/v3 双接受、源验签失败回退、密钥变更提示、合并优先级单测；
  - `umanager-helper`：验签链正/反用例（坏 sourceEndorsement / 坏源签名 / id 不在源目录 / 中央源旧路径兼容）；
  - `umanager-plan`：v3 payload 的 planId/有效性/字段完整性；
  - UI：软件源列表状态渲染 + 开关持久化。

## 10. 开放问题

- 同一 `applicationId` 出现在**多个源**（重复上架）：v1 规则「注册表顺序，前面的源胜出」，
  是否需要在源上架时做冲突预检（CI 发布时发现重复 id 即告警）——倾向：CI 告警 + 文档约定。
- 第三方源的 `catalogJson` 里的 hosts 白名单是否允许通配（现有 `*.<domain>` 窄例外语义）：建议沿用，
  但仍要求「下载 URL 由厂商签名或由 App 端 `source_engine` 校验」的现有防线不因多源松动。
- `sourceEndorsement` 是否需要有效期（配合计划 15 分钟）；倾向：随计划有效期即可，不加额外轮子。
- `/feed.json`（v2 聚合）双轨保留多久：随 0.9.1 上线后观察旧 App 使用情况，弃用后从 CI 移除发布；
  移除时不需要给 App 发版（旧 App 读不到 feed 自然走缓存/降级——个人项目可接受）。