# UManager 架构与完成说明

> 本文是 UManager「软件信息抓取搬进 CI / GitHub Pages」这一系列改造的完成说明与架构备忘。面向后续维护者与 AI 编程 agent。当前版本：`v0.2.0`。

## 1. 项目是什么

UManager 是面向 Ubuntu 的个人软件管家（Tauri 2 + React + Rust），管理从厂商官网/官方仓库安装的 `.deb` 应用及其版本更新、安装、卸载。所有系统级操作都经过一个独立的特权 helper（Polkit），并遵循「不可变计划 + 特权 dry-run + 再次确认」的安全模型（Polkit 用 `auth_admin_keep`：一次会话只弹一次管理员授权，之后该会话内的后续特权操作不再重复弹窗）。

## 2. 核心架构：软件信息只依赖签名 feed

历史版本中，UManager 在用户机器上抓取官网、调用 GitHub Releases API、解析本机 `apt-cache policy` 来获知候选版本。这些逻辑已全部移除。现在的模型是：

- **`feed.json` 是软件信息的唯一来源**（候选版本、大小、SHA-256、下载地址、发布 tag、展示版本）。
- feed 由 GitHub Actions 定时/按需抓取厂商来源后生成，**用 Ed25519 签名**发布到 GitHub Pages。
- App 只拉取 `feed.json` + `feed.json.sig`，验签、校验 HTTPS 精确域名与字段格式后使用。
- 每次验签成功后把 `feed.json` 原文 + 签名原子写入应用缓存（`feed/feed-cache.json`）；之后读取优先本地缓存，每次用内置公钥重新验签，过期则 stale-while-revalidate（先返回缓存、后台刷新，30 分钟周期 + 设置页手动刷新）。
- 真正的安装路径不变：按 feed 中的下载地址下载 `.deb` → 核对域名、大小、SHA-256、`.deb` 包名/版本/架构 → 生成不可变计划 → 特权 helper 复核 → 固定 `dpkg --install` / `dpkg --remove`。

发布地址：

```text
https://null-object-0000.github.io/umanager/feed.json
https://null-object-0000.github.io/umanager/feed.json.sig
```

### 2.1 feed.json 结构（schemaVersion 2）

```jsonc
{
  "schemaVersion": 2,
  "generatedAtUnixSeconds": 1750000000,
  "applications": {
    "vscode": { "packageName": "code", "version": "…", "architecture": "amd64",
                "size": …, "sha256": "…", "downloadUrl": "…", "websiteVersion": "…" }
  },
  "catalogJson": "[{\"applicationId\":\"example\", …}]",  // 签名目录（新增软件）
  "catalogSignature": "<hex Ed25519 over catalogJson>",
  "selfUpdate": { "packageName": "u-manager", "version": "…", "size": …,
                  "sha256": "…", "downloadUrl": "…", "releaseTag": "v…", "assetName": "…" },
  "developmentTools": { "claude-code": { "npmPackage": "…", "version": "…" } }
}
```

- `applications`：所有受管应用的版本数据（版本会变的部分）。
- `catalogJson` / `catalogSignature`：**新增软件**的完整定义（含允许域名、是否可卸载、下载策略），单独签名，供特权 helper 授权。
- `selfUpdate`：UManager 自更新来源。
- `developmentTools`：CLI 开发工具（Claude Code / OpenCode / Pi / Codex）的最新版本。

## 3. 两层清单：静态契约 vs 动态数据

| | 文件 | 谁消费 | 改了要不要更新 App |
|---|---|---|---|
| 内置基础清单 | `src-tauri/resources/vendors.json` | 主程序 + 特权 helper（编译进二进制） | 要发新版 |
| 动态版本数据 | feed `applications` | 主程序运行时 | 不用 |
| 新增软件清单 | `feed-sources.json`（仓库，CI-only）→ feed `catalogJson` | 主程序合并 + helper 验签授权 | **不用** |

`vendors.json` 的 `metadataFeed` 字段声明 feed 的 URL 与精确域名白名单。

## 4. 新增软件（无需更新 App）

在仓库根目录 `feed-sources.json` 的 `applications` 里追加一条与 `vendors.json` 同构的记录，push 即可：

1. CI（`update-feed.yml`）重新抓取，把该软件写进 feed 的 `applications` 与 `catalogJson`，并分别用 `FEED_SIGNING_KEY` 签名；
2. 老版本 App 拉取新 feed，主程序把 `catalogJson` 中的新应用合并进软件列表；
3. 安装/卸载该新软件时，主程序把 `catalogJson` + `catalogSignature` 写进不可变计划，特权 helper 用内置公钥验签后授权。

`feed-sources.json` 中每条记录与 `vendors.json` 的 `applications` 同构：
- `aptRepository`：需补 `packagesIndexUrl`（指向该仓库的 Debian `Packages` 索引，供 CI 解析候选版本）；
- `releaseApi`：需补 `releaseApiUrl` / `assetNamePattern` / `stripTagPrefix` / `assetDownloadHosts`；
- `stableDownloadEndpoint`：需补 `officialPageUrl` / `downloadUrl` / `pageVersionMarker` / `downloadLinkFileName` / `pageVersionSegments`。

## 5. 签名与信任边界

- **私钥**：只存在 GitHub Actions secret `FEED_SIGNING_KEY`（Ed25519），绝不进入仓库、不随 App 发布。
- **公钥**：内置在主程序 `src-tauri/src/feed.rs` 与特权 helper `crates/umanager-helper/src/main.rs`（同一公钥，hex `57d369…c8f9`）。
- **两处签名**：
  1. `feed.json.sig`：对 `feed.json` 原文签名，主程序校验（保证版本数据可信）；
  2. `catalogSignature`：对 `catalogJson` 原文签名，特权 helper 校验（保证新增软件的安装/卸载授权可信）。
- **helper 不信任任意计划字段**：feed 新增软件的授权来自「内置公钥 + 计划内携带的已签名 `catalogJson`」，而不是放宽编译期白名单。helper 仍逐条复核：计划完整性（SHA-256）、`catalogJson` 验签、包名/架构/是否可卸载、`.deb` 元数据与 SHA-256、本机安装状态，最后才执行固定 `dpkg` 命令。

> 信任说明：APT 类软件从「厂商 GPG 签名索引」变为「UManager Ed25519 签名 feed + HTTPS + 哈希」。`.deb` 下载仍锁厂商精确域名并做 SHA-256 + 包元数据复核。

## 6. 关键文件地图

| 文件 | 职责 |
|---|---|
| `.github/workflows/update-feed.yml` | 定时/手动/相关文件变更时抓取、签名、发布 feed 到 Pages |
| `.github/workflows/release.yml` | 推送 `v*` tag 时构建 `.deb` 并发布 Release |
| `scripts/update-feed.mjs` | feed 生成器：抓取厂商来源 + Ed25519 签名 |
| `feed-sources.json` | 新增软件的清单（CI-only，不编译进 App） |
| `src-tauri/resources/vendors.json` | 内置基础清单 + `metadataFeed` 配置 |
| `crates/umanager-catalog/src/lib.rs` | 清单/来源/feed 配置的数据模型 |
| `crates/umanager-plan/src/lib.rs` | 不可变计划 schema（v2，含签名目录字段） |
| `crates/umanager-helper/src/main.rs` | 特权 helper：白名单、验签、固定 dpkg 命令 |
| `src-tauri/src/feed.rs` | feed 拉取/验签/磁盘缓存 + SWR + 后台刷新/合并新增软件/状态 |
| `src-tauri/src/source_engine.rs` | feed → `ApplicationDetails`/`DownloadPlan` + 下载校验 |
| `src-tauri/src/scanner.rs` | 本机已安装包扫描（候选版本由 feed 填） |
| `src-tauri/src/operation_plan.rs` | 安装/卸载/自更新计划的生成 |
| `src-tauri/src/installable.rs` | 软件商店可安装列表 |
| `src-tauri/src/lib.rs` | Tauri command 入口 |
| `src-tauri/src/local_deb.rs` / `installation.rs` | 本地 `.deb` 导入 / 安装形态检测 |
| `src-tauri/src/dev_tools.rs` / `dev_cli_tools.rs` | nvm/rustup 工具链 / CLI 开发工具 |
| `src/App.tsx` / `src/api.ts` / `src/types.ts` | 前端 |

## 7. 构建与测试

```bash
# Rust
cargo check --manifest-path src-tauri/Cargo.toml
cargo test  --manifest-path src-tauri/Cargo.toml
cargo test  --manifest-path crates/umanager-catalog/Cargo.toml
cargo test  --manifest-path crates/umanager-plan/Cargo.toml
cargo test  --manifest-path crates/umanager-helper/Cargo.toml

# 前端
npm test
npm run build

# 本地生成 feed（需网络；设置 FEED_SIGNING_KEY 才签名）
npm run update-feed
```

发布：bump 三处版本（`package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`）→ commit → `git tag -a vX.Y.Z` → push tag，`release.yml` 自动构建 `.deb`。

## 8. 关键不变量（维护/修改时必须遵守）

1. 软件版本信息**只来自签名 feed**，不要在 App 里重新引入本机抓取。
2. `vendors.json` 编译进主程序与 helper；改它=要发新版。
3. 新增受管软件只改 `feed-sources.json`；不要把手写条目塞进 feed 而绕过签名。
4. helper 只通过「内置公钥 + 计划内已签名 `catalogJson`」授权 feed 新增软件；不信任未签名字段。
5. 所有下载域名都是精确白名单，禁止通配/前缀匹配。
6. 系统命令一律固定 argv（`dpkg-query`/`dpkg-deb`/`dpkg --compare-versions`/`dpkg --install`/`dpkg --remove`），不经过 shell，不拼接用户输入。
7. 计划不可变：payload SHA-256 作为 plan_id，15 分钟有效期，只读、归属当前用户。
8. 私钥只存在于 `FEED_SIGNING_KEY` secret；任何提交都不应包含私钥。
9. feed schema 与 plan schema 当前都是 v2；升级时主程序、helper、generator 三处要同步。
