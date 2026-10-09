# DESIGN-app-variants.md — 应用变体切换（国内版 / 国际版、稳定版 / 开发版）

> 目标：同一款软件的多条版本线（Qoder / Trae / 飞书 / 滴答清单的国内版 / 国际版、
> Wine 的稳定版 / 开发版）在商店里是**一张卡片**，并且能像「开发环境」里的 dsh 版本
> 线一样，在**软件详情页**用分段控件切换，安装 / 更新 / 卸载都作用于所选变体。
> 可共存的变体（国内版 / 国际版）点一下就换；**互斥**的变体（Wine 稳定版 / 开发版）
> 走「先卸载、再安装」的替换流程，见 §7。

## 1. 背景

Qoder 与 Trae 都存在两个互相独立的发行版：

| 产品 | 国内版（包名） | 国际版（包名） |
|---|---|---|
| Qoder | `qoder-cn`（qoder.cn，北京 OSS） | `qoder`（qoder.com，GA 加速） |
| Trae | `trae-cn`（trae.cn，`lf-cdn.trae.com.cn`） | `trae`（trae.ai，`lf-cdn.trae.ai`） |


两者的账号、Credits、模型、部署区域互不相通（见[阿里云说明](https://developer.aliyun.com/article/1768159)），
包名也不同（`Conflicts` 只与自身冲突），因此**可以同时安装**，也必须由用户显式选择要装哪一个。

把它们做成两条独立目录条目（本设计之前的状态）在功能上已经可用，但商店里会出现两张
同名卡片，用户看不出「这两个是同一款软件的两个版本」。

## 2. 方案选择：展示层折叠，而不是 feed 级 channels

dsh 的「版本线」（latest / alpha / next）实现在**一条** `developmentTools` 条目里：feed 给出
`channels: { tag: version }`，App 侧持久化所选 tag，安装时拼 `npm install -g pkg@version`。

把这套照搬成「应用级 channels」（一条 `applications` 条目内含多个 channel，各自带
`packageName` / `downloadUrl` / `sha256`）看起来更「像 dsh」，但代价落在安全关键路径上：

- **特权 helper 的授权链**（AGENTS 不变量 4/5）只信任「内置公钥 + 计划内已签名
  `catalogJson`」里**单条**记录：它按 `applicationId` 取出该应用、核对
  `packageName` / 大小 / SHA-256，并检查下载 URL 是否落在该记录声明的 `downloadHosts` 里。
  单条目多 channel 意味着计划要新增 `channel` 字段、helper 要按 channel 重新选取并复核，
  等于**改动授权语义**；
- 而两套变体的包名本来就不同 → 安装状态、更新检测、卸载、SHA-256 校验、下载队列本来就是
  按 `packageName` 分开的，**现有链路一条都不用改**。

因此采用：**每个变体保持一条完整、可独立授权的目录条目**，只在展示层增加分组元数据，
把同组条目折叠成一张卡片 + 详情页切换控件。这是一次 UI/元数据改动，不触碰下载、计划、
helper 与 `dpkg` 调用。

### 向后兼容

`variantGroup` 等字段全部可选。老版本 App 不认识它们 → 只会看到两个普通条目（今天的行为），
不会崩、不会误装；若把包名藏进单条目的 channels 里，老 App 会直接丢掉这个应用。

## 3. 数据模型

三个字段（`feed-sources.json` → 签名 `catalogJson` → App）：

| 字段 | 含义 |
|---|---|
| `variantGroup` | 组 id；同组条目是同一产品的不同市场版本 |
| `variantLabel` | 切换控件上的短标签，如「国内版」「国际版」 |
| `variantDefault` | 组内没有任何变体被安装时默认展示哪一个（每组恰好一个） |

落点：

- `feed-sources.json`：Qoder / Trae 各两个条目声明这三个字段；`displayName` 统一为产品名
  （`Qoder` / `Trae`），由 `variantLabel` 区分市场版本；
- 生成器 `scripts/update-feed.mjs`：`toCatalogApplication()` 是整对象透传（只剥 CI-only 的
  `gatewayUrl` / `endpointHeaders` / `immutableDownloadUrl` 等），**不需要白名单**；
- `crates/umanager-catalog/src/lib.rs`：`Application` 增加三个可选字段（`#[serde(default)]`）。
  全仓库没有 `deny_unknown_fields`，因此新老双向兼容；
- `src/types.ts` / `src/variants.ts`：前端类型与纯函数。

## 4. 展示层规则

### 4.1 主变体（决定卡片显示什么）

`pickVariant()` 的优先级（表见下）写死在纯函数里并单测覆盖：

| 优先级 | 规则 | 理由 |
|---|---|---|
| 1 | 用户上次选择**且已安装** | 两个变体都装着时，尊重用户最近的选择 |
| 2 | 任一已安装的变体 | 卡片必须反映本机真实状态，不能因为用户切过一次就把已装版本显示成「未安装」 |
| 3 | 用户上次选择的变体 | 什么都没装时，沿用上次浏览的那个 |
| 4 | `variantDefault`（缺失则组内第一个） | feed 声明的默认，如国内市场的「国内版」 |

排序同样是纯函数：默认变体在前，其余按 `applicationId` 稳定排序 —— 切换控件的顺序不依赖
feed 顺序。

### 4.2 详情页切换

- 复用 dsh 版本线的 UI 与样式（`.channel-picker` + `.filter-tabs.segmented`），标签「版本」，
  提示文案说明「两份互相独立的安装包，可以同时安装」；
- **不引入新状态**：抽屉当前展示的是哪个 `applicationId`，切换控件就作用于那一个。点另一个
  变体 = 把当前抽屉换成那个变体的安装包（已安装 → `UpdateDrawer`，未安装 → `InstallDrawer`）；
- 选择写入 `localStorage["umanager.app-variant.<group>"]`（本机 UI 偏好，不进签名 feed、
  不进特权计划，与商店排序偏好同一处理方式）；
- 列表卡片副标题显示「分类 · 变体标签」，搜索也会匹配变体标签（搜「国际版」能找到卡片）。

### 4.3 折叠的影响面

`softwareItems` 折叠发生在所有下游逻辑之前，因此更新计数、更新页、分类筛选、排序、
「全部更新」候选都自动按「一张卡片 = 一款软件」的口径工作。

## 5. 安全不变量核对

| 不变量 | 是否受影响 |
|---|---|
| 1 软件信息只来自签名 feed | 否，分组字段同样来自签名 `catalogJson` |
| 2 `vendors.json` 编译期事实来源 | 否，未改动 |
| 3 新增受管软件只改 `feed-sources.json` | 是同一路径；本次只加字段 |
| 4 helper 只信任内置公钥 + 计划内已签名 `catalogJson` | 否。计划仍携带**单条**记录，helper 复核逻辑未改 |
| 5 下载域名精确白名单 | 否。每个变体各自的 `downloadHosts` 不变，切换只是换一条已签名记录 |
| 6 下载只经后台队列 | 否，未改动 |
| 7 固定 argv、不经 shell | 否，未改动 |
| 8 计划不可变、15 分钟有效期 | 否，未改动 |
| 9 私钥只在 GitHub secret | 否，未改动 |
| 10 schema v2 同步 | 否。新增字段全部可选，`Application` 的 JSON 仍是 v2 形状 |

## 6. 限制（已知取舍）

1. **变体通常有不同包名**，且默认假设它们**可以共存**（国内版 / 国际版就是如此）。
   包名相同、只是 stable / beta 双下载地址的需求仍然表达不了 —— 那种才需要真正的
   feed 级 channels。**互斥**的两条版本线（如 Wine 稳定版 / 开发版）用
   `variantExclusive` 表达，见 §7。
2. **变体必须在同一个签名源里**（都来自 `feed-sources.json`），不能跨源分组。
3. **单成员组不显示切换控件**（`isSwitchableGroup`）；组只有一个变体时按普通条目渲染。
4. 折叠是展示层的，所以两个变体**可以同时安装**；卡片只代表「主变体」，另一个变体通过
   详情页切换查看。（互斥组例外：两者装不到一起，见 §7。）

## 7. 互斥变体：Wine 稳定版 / 开发版

### 7.1 为什么不能照搬国内版 / 国际版

WineHQ 仓库里 `winehq-stable` 与 `winehq-devel` 的元数据是：

```
Provides: wine, wine-amd64, wine-i386, …
Conflicts: wine, wine-amd64, wine-i386
```

两个包都提供并冲突于 `wine`，dpkg 语义下**不能共存**（`wine-stable` /
`wine-devel` 运行时包同理）。因此「切换」只能是**替换**：先卸载已安装的那一条版本线，
再安装目标版本线。国内版 / 国际版的「点一下就换抽屉」在这里会直接撞上 dpkg 的
Conflicts 报错。

### 7.2 数据模型

只新增一个可选字段（同样由 `feed-sources.json` → 签名 `catalogJson` → App 透传）：

| 字段 | 含义 |
|---|---|
| `variantExclusive` | 组内变体互斥、不能共存；商店用「替换」而不是「另装一个」 |

落点：`feed-sources.json`（`wine` / `wine-stable` 两条）→ `scripts/update-feed.mjs`
（整对象透传）→ `crates/umanager-catalog/src/lib.rs`（`#[serde(default)] bool`）→
`src/types.ts` / `src/variants.ts`（纯函数 `isExclusiveGroup` / `conflictingVariant`）。

**判定宁可保守**：只有组内**每个**成员都声明 `variantExclusive` 才按互斥处理，漏标一个
就退化成普通变体组（不会凭空出现「需要先卸载」的提示）。

### 7.3 交互：一键替换，两次授权

1. 详情页切到未安装的另一个版本线：抽屉里给出黄色提示「A 和 B 不能同时安装」，主操作
   从「获取」变成「卸载 A」；
2. 点它 → 打开**原有**的卸载对话框（不可变计划 → 特权复核 dry-run → 二次确认后
   `dpkg --remove`）；
3. 卸载成功后自动关闭该对话框，打开目标版本线的安装抽屉并**排队下载**；
4. 下载完成后由用户点「安装」→ 原有的安装计划 → 特权复核 → 二次确认。

也就是「一次点击发起替换」，但**没有任何特权操作被自动执行**：卸载与安装各自保留
dry-run + Polkit 授权 + 二次确认，`helper` 仍然只按单条签名记录复核。

### 7.4 已知限制

- 卸载 / 安装的对象是 WineHQ 的**元包**（`winehq-stable` / `winehq-devel`，各约 1 KB），
  真正的 `wine-stable` / `wine-devel` 运行时包由 apt 按依赖解析；替换后 dpkg 可能报
  「依赖未满足」，helper 会按既有逻辑提示用 `sudo apt-get install -f` 让 apt 收尾
  （apt 会顺带移除冲突的那条版本线）。这与 Wine 出现在 UManager 里的方式一致，不是本次
  新增的行为。
- 互斥组只有两条版本线时才算「切换」（staging 若日后加入，同样适用）。

## 8. 测试覆盖

| 位置 | 覆盖 |
|---|---|
| `src/variants.test.ts` | 分组、排序、主变体优先级、持久化键、折叠、互斥组与冲突判定 |
| `scripts/feed-variant-groups.test.mjs` | `feed-sources.json` 的变体元数据（成员数、标签唯一非空、恰好一个默认、包名唯一、`displayName` 一致、Wine 组互斥且默认稳定版） |
| `crates/umanager-catalog/src/lib.rs` | 三个字段可选（老 catalog 可解析）+ 序列化回 camelCase + `variantExclusive` 缺省为 false |
