# UManager 架构与完成说明

> 本文是 UManager 的架构与实现说明，面向后续维护者与 AI 编程 agent。当前版本：`v0.8.5`。

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
                "size": …, "sha256": "…", "downloadUrl": "…", "websiteVersion": "…" },
    "flclash": { "packageName": "flclash", "version": "…", "architecture": "amd64",
                 "size": …, "sha256": "…", "downloadUrl": "…", "releaseTag": "v…",
                 "websiteVersion": "…", "releaseNotes": "…", "releaseNotesUrl": "https://…" }
  },
  "catalogJson": "[{\"applicationId\":\"example\", …}]",  // 签名目录（新增软件）
  "catalogSignature": "<hex Ed25519 over catalogJson>",
  "selfUpdate": { "packageName": "u-manager", "version": "…", "size": …,
                  "sha256": "…", "downloadUrl": "…", "releaseTag": "v…", "assetName": "…" },
  "developmentTools": { "claude-code": { "npmPackage": "…", "version": "…" },
                        "hermes": { "npmPackage": null, "version": "…" } }
}
```

- `applications`：所有受管应用的版本数据（版本会变的部分）。
- `catalogJson` / `catalogSignature`：**新增软件**的完整定义（含允许域名、是否可卸载、下载策略），单独签名，供特权 helper 授权。
- `selfUpdate`：UManager 自更新来源。
- `developmentTools`：CLI 开发工具（Claude Code / OpenCode / Pi / Codex / DSH / Hermes）的最新版本；`npmPackage` 可为空（非 npm 分发的工具，版本由 `feed-sources.json` 的 `toolVersionOverrides` 解析）。

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
| `scripts/update-feed.mjs` | feed 生成器：抓取厂商来源 + Ed25519 签名；支持 `--group/--partial/--merge` 多源分组模式（DESIGN-multi-source.md §7） |
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
5. 所有下载域名都是精确白名单，禁止通配/前缀匹配。唯一例外：`*.<domain>`（如飞书 `*.feishucdn.com`、腾讯文档 `*.docs.qq.com`）允许该域名及其子域，仅限 CDN 分片主机名会漂移的厂商，实现与语义见 AGENTS.md。
6. 系统命令一律固定 argv（`dpkg-query`/`dpkg-deb`/`dpkg --compare-versions`/`dpkg --install`/`dpkg --remove`），不经过 shell，不拼接用户输入。
7. 计划不可变：payload SHA-256 作为 plan_id，15 分钟有效期，只读、归属当前用户。
8. 私钥只存在于 `FEED_SIGNING_KEY` secret；任何提交都不应包含私钥。
9. feed schema 与 plan schema 当前都是 v2；升级时主程序、helper、generator 三处要同步。

## 9. 内置软件源（vendors.json）与 source kind

`src-tauri/resources/vendors.json` 编译进主程序和特权 helper，是 UManager「认识哪些软件、从哪里下载、允许哪些域名、能否卸载」的编译期事实来源。新增一个内置软件，只需在 `applications` 里追加一条记录；新增 **feed 下发**的软件（免发版）则在仓库根目录的 `feed-sources.json` 里追加。

`applications` 常见字段：`applicationId` / `packageName` / `displayName` / `vendor` / `architecture`（标识与展示）、`homepage` / `icon` / `accentColor`（可选）、`description`（列表与详情的一行描述）、`removable`（是否允许卸载）、`source`（下载策略）。

`source.kind` 分为五类：

| kind | 含义 | 例子 |
|---|---|---|
| `aptRepository` | 从官方 APT 仓库 `Packages` 索引解析候选版本、大小与 SHA-256 | VS Code、Google Chrome、ChatGPT Desktop、GitHub CLI、Microsoft Edge |
| `stableDownloadEndpoint` | 从官网固定地址下载，并从官网页面解析展示版本 | 微信、Bitwarden、腾讯文档 |
| `releaseApi` | 从 GitHub Releases API 选择匹配资产，读取 `sha256:` 摘要 | FlClash、LocalSend |
| `versionEndpoint` | 从官网动态「版本 + 下载地址」接口解析，可带签名/限时地址子步骤 | QQ、QQ 音乐、腾讯会议、WPS Office、Obsidian、飞书 |
| `browserImport` | 仅登记本机识别与卸载，不在商店提供自动下载 | 钉钉 |

`source` 中声明的所有 `*Hosts` 都是精确域名白名单：下载与 HTTP 重定向都只接受这些域名，拒绝相似域名。通用下载引擎（`src-tauri/src/source_engine.rs`）与 helper 的白名单校验都从同一份内置 JSON 生成，新增软件不需要改 Rust 代码或新增 Tauri command。`aptRepository` 类来源还可选声明 `packagesIndexUrl`（仓库 `Packages` 索引），仅供 CI 解析候选版本，应用本身不依赖该字段。

## 10. 中央元数据源运行细节

- **触发**：`.github/workflows/update-feed.yml` 定时（每 6 小时）、手动触发，以及 `vendors.json` / `feed-sources.json` / `scripts/update-feed.mjs` / workflow 变更时；
- **分组抓取**：按 `feed-sources.json` 的 `sources` 注册表 + 每个应用的 `sourceGroup`（内置应用的归属在脚本内置表里，wechat/wemeet → tencent）把来源拆成并行 job（当前 `tencent` / `common` 两组）。每个 job 用 `--group <id>` 只抓本组应用，**就地完成**上一版兜底与 version-time 合并，产出本组**最终签名源 feed**（`feed.<group>.json`，含本组 `catalogJson`/`catalogSignature`）；组内抓取有界并发（默认 5），`.deb` 只下载一次、图标不变时跳过下载；
- **合并发布**：`merge` job 用 `--merge` 发现所有源 feed，按 `feed-sources.json` 顺序聚合 `applications` / `catalogJson`；某组 job 挂掉时该组应用回落到上一版中央 feed 条目；`selfUpdate` 与开发工具是**中央独有数据**，由 merge 就地抓取合并；最后把各源 feed（+.sig）复制进发布目录、签名中央 feed。新增源只需注册表加一行 + 个别应用标 `sourceGroup` + matrix 数组加一个 id；默认不带参数的 `npm run update-feed` 仍是单 pass 全量模式（本地/兜底）；
- **签名**：用 GitHub Actions secret `FEED_SIGNING_KEY` 对 `feed.json` 原文做 Ed25519 签名产出 `feed.json.sig`，并对 `catalogJson` 单独签名（`catalogSignature`）供特权 helper 授权 feed 新增软件；各独立源 feed（`feed.tencent.json` / `feed.common.json`）一期暂用同一把私钥签名，第二期随 schema v3 换成各源独立密钥（DESIGN-multi-source.md §8）；
- **发布**：`configure-pages` + `upload-pages-artifact` + `deploy-pages` 部署到 GitHub Pages，地址为 `https://<owner>.github.io/<repo>/feed.json`。要让地址生效，需在 Settings → Pages 的 Source 选择「GitHub Actions」，或直接跑一次 `update-feed`（`configure-pages` 会自动启用 Pages）。

应用侧：每次验签成功后，feed 原文 + 签名原子写入缓存（`feed/feed-cache.json`）；之后读取优先本地缓存（每次用内置公钥重新验签），过期时先返回缓存、后台再刷新；后台任务每 30 分钟检查一次，设置页提供「立即刷新」。断网或拉取失败时，只要缓存可用，应用仍展示最近一次成功的数据，并在「设置 → 软件信息源」标注“本地缓存 + 失败原因”。应用会校验 feed 的 HTTPS 精确域名、大小上限、Ed25519 签名与字段格式；失败且无缓存时对应应用显示为不可用。

## 11. 开发环境：用户级工具链与 CLI 工具

「开发环境」页分三部分：运行时工具链与 CLI AI 编程工具走无特权链路（当前用户身份、不经过 Polkit helper、不请求 root）；「系统级 CLI 工具」区块（如 GitHub CLI `gh`）则通过白名单 `.deb` 特权链路管理安装在系统层的命令行工具。

**运行时工具链**（`src-tauri/src/dev_tools.rs`）：通过用户级版本管理器管理。`vendors.json` 的 `developmentToolchains` 数组登记一条记录即可接入：`toolchainId` / `displayName` / `vendor` / `homepage`、`manager`（如 `nvm`、`rustup`）、`managerKind`（`shell` 需 source 脚本 / `binary` 直接执行）、`managerHome`、`managerScript` / `managerBinary`、`versionsDirectory`。当前内置 Node.js（nvm，shell）与 Rust（rustup，binary）两条工具链，支持“安装并设为默认”“设为默认”“卸载”。nvm 通过 `/bin/bash -c 'source nvm.sh --no-use; nvm …'` 执行；rustup 直接以参数向量调用，两者都只由固定子命令 + 严格校验的版本/别名 token 组成。

**CLI AI 编程工具**（`src-tauri/src/dev_cli_tools.rs`）：管理单版本、用户级安装的工具。`developmentTools` 数组登记字段：`toolId` / `displayName` / `vendor` / `homepage` / `accentColor`、`binaryName`、`npmPackage`（可为空，非 npm 分发的工具留空）、`distTag`（可选，npm 工具跟踪的 dist-tag 通道：`latest` / `alpha` / `next`，默认 `latest`；feed 版本解析与 App 的 npm 安装/更新都走该标签）、`installer.kind`（`npm` 或 `curlScript`）、`uninstall.kind`（`npm` / `removeFiles` / `selfCommand`）、`update.kind`（`selfCommand` 或回退重跑 installer）。当前内置：

| 工具 | npm 包 | 安装方式 | 命令 |
|---|---|---|---|
| Claude Code | `@anthropic-ai/claude-code` | 官方脚本 `https://claude.ai/install.sh` | `claude` |
| OpenCode | `opencode-ai` | 官方脚本 `https://opencode.ai/install` | `opencode` |
| Pi | `@earendil-works/pi-coding-agent` | 官方脚本 `https://pi.dev/install.sh` | `pi` |
| Codex CLI | `@openai/codex` | npm 全局 | `codex` |
| DeepSeek Harness | `@deepseek-ai/dsh` | npm 全局 | `dsh` |
| Hermes Agent | —（git/Python） | 官方脚本 `https://hermes-agent.nousresearch.com/install.sh`，更新 `hermes update`、卸载 `hermes uninstall --yes` | `hermes` |

检测同时识别三种安装来源：npm 全局、官方安装器（`~/.local/bin` / `~/.opencode/bin`）与 PATH 上的可执行文件；最新版本统一从签名 feed 读取（npm 工具走 npm registry，非 npm 工具由 `feed-sources.json` 的 `toolVersionOverrides` 从厂商 GitHub Releases 解析，如 Hermes 从发布标题 `Hermes Agent v0.21.0 …` 提取版本）。`curlScript` 型安装以当前用户身份执行厂商官方域名上的 HTTPS 安装脚本，UManager 只固定 URL 与参数，不接收任意 shell，但无法证明脚本内容与发布者身份。

## 12. 历史适配器记录（VS Code / 微信 / FlClash）

以下各节描述历史行为；候选版本、大小与 SHA-256 现已改由中央元数据源提供，应用不再本机解析。仅保留信任边界与解析路径备忘。

- **VS Code**：历史 aptRepository 适配器校验包名 `code`、架构 `amd64`，精确匹配 `packages.microsoft.com`，未发现可信仓库时回退官方 stable 下载接口。
- **微信**：官网三段版本号 + 固定 x86_64 `.deb` 地址；校验 `linux.weixin.qq.com`、`dldir1v6.qq.com` 精确域名；用 HTTP Range 读 4 KiB 解析 Debian ar 目录并精确截取 `control.tar.*`（硬限制 4 MiB），再取包名 `wechat`、完整版本、`amd64`。官网与控制区并行请求，远端元数据进程内缓存 15 分钟。官网未发布独立签名 SHA-256 清单，该哈希只保证“下载完成后到安装前文件未变化”。
- **FlClash**：读取 `api.github.com/repos/chen08209/FlClash/releases/latest`，精确匹配 `FlClash-<tag>-linux-amd64.deb`；下载允许 `github.com` 及重定向到 `objects.githubusercontent.com` / `release-assets.githubusercontent.com`；同样用 Range 探测 + `dpkg-deb --field` 读取包元数据。GitHub 资产摘要比微信多一层发布者哈希，但仍不是签名 APT 索引级别。

## 13. 安装 / 卸载 / 自更新的执行细节

**本地 `.deb` 安装**：Linux 会把 `.deb` 关联到 UManager。应用先无特权读取包名/版本/架构并计算大小与 SHA-256，标记为“用户提供，来源未验证”，拒绝同版本重装、降级和不兼容架构；以不覆盖方式复制到 `imports` 缓存并重新校验；用户确认后生成 15 分钟有效的不可变计划，经 helper dry-run 复核，再次确认后以固定 `/usr/bin/dpkg --install <root-owned-staged-deb>` 执行。

**安装 / 卸载**：下载后复核 HTTPS 精确域名、重定向、大小、SHA-256 与 `.deb` 包元数据；helper 在 dry-run 与真正执行前都会再次核对安装状态、系统架构、计划与官方源记录。卸载走独立不可变计划，仅执行白名单中固定的 `dpkg --remove`，不 `purge`、不自动移除依赖、不删除用户主目录数据。UManager 自身卸载也不再是“设置”里的独立入口：`selfUpdate` 源渲染的 `u-manager` 应用 `removable: true`，因此作为受管软件出现在“软件 / 更新”页，与其他软件共用同一个 `RemovalDialog` 与同一套 `create_removal_operation_plan` / `run_removal_dry_run` / `remove_managed_package` 命令；`create_removal_operation_plan` 对 `u-manager` 分发到 `create_self_removal_plan`，`resolve_removal_action` 再按计划里的 `removeUmanager` 动作路由到 helper 的 `remove-umanager`。

**UManager 自更新**：不再是“设置”里的独立入口，而是作为受管软件的一员出现在“软件 / 更新”页，复用与其他软件完全一致的下载 → SHA-256 校验 → 不可变计划 → 特权复核 → 安装流程。`selfUpdate` 源在 `require_application` 里被解析成普通 `releaseApi` 应用，走同一套 `get_application_details` / `download_package` / `create_operation_plan` / `run_operation_dry_run` / `install_package` 命令；`create_operation_plan` 对 `umanager` 分发到 `create_self_update_plan`，生成 `installSelfUpdate` 计划，helper 用 `install-umanager` 动作复核“必须比当前版本新、必须是 `.deb` 安装版”后执行。安装完成后抽屉按钮显示为“重启 UManager”（调用 `restart_app`）。

helper 只接受固定动作（均支持 `--dry-run` 与 `--execute`）：

| 来源 | 动作 |
|---|---|
| `aptRepository` | `install-verified-deb` |
| `stableDownloadEndpoint` / `releaseApi` | `install-verified-website-deb` |
| 用户本地包 | `install-local-deb` |
| `removable: true` 的受管软件 | `remove-managed-package` |
| UManager 自身 | `remove-umanager` |

helper 的这几张白名单同样从内置 JSON 生成，并重新检查计划完整性、有效期、白名单、当前安装状态，以及安装包的缓存路径、文件归属、元数据、官方仓库记录、大小和 SHA-256，全程不经过 shell。

## 14. 安全边界

React 只调用类型明确的 Tauri commands。Rust 端仅以固定参数调用 `dpkg-query`、`apt-cache`、`dpkg-deb` 和 `dpkg --compare-versions`，不经过 shell，也不接受前端传入的命令或包名。`.deb` 打包配置将 helper 安装到 `/usr/libexec/umanager-helper`，并把 Polkit policy 安装到 `/usr/share/polkit-1/actions/`。

Polkit 授权使用 `auth_admin_keep`：一次「dry-run」特权复核会在当前登录会话内缓存管理员授权，因此紧随其后的「确认并安装/卸载」不会再次弹密码——一次操作只输入一次密码；但任何特权动作仍必须经由 `pkexec` 触发 helper、仍受固定白名单与计划复核约束。
