# AGENTS.md — UManager 开发指引

本文件给在 UManager 仓库里工作的 AI 编程 agent（以及人类协作者）提供最低限度的上下文。详细架构与背景见：

- **架构与实现说明**：[`ARCHITECTURE.md`](ARCHITECTURE.md)
- **用户文档 / 贡献者快速开始**：[`README.md`](README.md)

## 项目速览

UManager 是面向 Ubuntu 的个人软件管家：**Tauri 2（Rust 后端 + React 前端）**，管理从厂商官网/官方仓库安装的 `.deb` 应用。系统级安装/卸载全部经**特权 helper**（Polkit，`crates/umanager-helper`）执行，采用「不可变计划 + 特权 dry-run + 再次确认」（Polkit 用 `auth_admin_keep`，一次会话只弹一次管理员授权）。

## 必须遵守的安全不变量（改动前先读）

1. **软件信息只来自签名 feed**（`feed.json` + `feed.json.sig`，发布在 GitHub Pages）。禁止在 App 里重新引入本机抓取（官网 HTML / GitHub API / `apt-cache policy`）。
2. **`src-tauri/resources/vendors.json` 编译进主程序和特权 helper**。改动它=需要发新版 App；它是「受管软件有哪些、允许哪些域名、能否卸载」的编译期事实来源。
3. **新增受管软件只改 `feed-sources.json`**（仓库根目录，CI-only）。CI 会把它抓取并 Ed25519 签名进 `catalogJson`，老版本 App 无需更新。
4. **特权 helper 只通过「内置公钥 + 计划内已签名 `catalogJson`/`catalogSignature`」授权 feed 新增软件**。不要放宽 helper 白名单去信任计划里的任意未签名字段。
5. **所有下载域名默认都是精确白名单**（`*Hosts`），禁止通配/前缀匹配。**唯一例外**：主机条目写成 `*.<domain>`（如飞书的 `*.feishucdn.com`）时，允许该域名及其子域——这是为 CDN 分片主机名会漂移的厂商（飞书 `lf?-ug-sign.feishucdn.com`）刻意保留的窄例外。`host_matches`（`src-tauri/src/source_engine.rs`）与生成器 `hostAllowedInList`（`scripts/update-feed.mjs`）实现同一语义，且必须同时更新。此例外不削弱其他防线：下载仍限 HTTPS、下载 URL 由厂商签名（`x-signature`/时间戳），`.deb` 仍按签名 feed 的 SHA-256 校验。
6. **系统命令固定 argv、不经 shell**：`dpkg-query`、`dpkg-deb`、`dpkg --compare-versions`、`dpkg --install`、`dpkg --remove`；不拼接用户输入。
7. **计划不可变**：`plan_id = SHA-256(payload)`，15 分钟有效期，只读、归属当前用户（`crates/umanager-plan`）。
8. **私钥只存在于 GitHub Actions secret `FEED_SIGNING_KEY`**；任何提交都不得包含私钥。公钥（hex `57d369…c8f9`）内置在 `src-tauri/src/feed.rs` 与 `crates/umanager-helper/src/main.rs`。
9. **schema 同步**：feed schema 与 plan schema 当前均为 **v2**。升级时主程序、helper、`scripts/update-feed.mjs` 三处必须同步修改。

## 关键文件（改哪里）

| 想做什么 | 文件 |
|---|---|
| 新增受管软件（免更新 App） | `feed-sources.json` |
| 改受管软件的基础定义/域名白名单 | `src-tauri/resources/vendors.json`（需发版） |
| feed 抓取/签名/发布 | `scripts/update-feed.mjs`、`.github/workflows/update-feed.yml` |
| 发布 `.deb` | `.github/workflows/release.yml`（推 `v*` tag） |
| 数据模型 | `crates/umanager-catalog/src/lib.rs` |
| 计划 schema | `crates/umanager-plan/src/lib.rs` |
| 特权 helper（白名单/验签/安装卸载） | `crates/umanager-helper/src/main.rs` |
| feed 拉取/验签/合并/状态 | `src-tauri/src/feed.rs` |
| 详情/下载计划/下载校验 | `src-tauri/src/source_engine.rs` |
| Tauri 命令入口 | `src-tauri/src/lib.rs` |

## 构建与测试

```bash
cargo check --manifest-path src-tauri/Cargo.toml
cargo test  --manifest-path src-tauri/Cargo.toml
cargo test  --manifest-path crates/umanager-catalog/Cargo.toml
cargo test  --manifest-path crates/umanager-plan/Cargo.toml
cargo test  --manifest-path crates/umanager-helper/Cargo.toml
npm test
npm run build
npm run update-feed   # 本地生成 feed；需网络，设置 FEED_SIGNING_KEY 才签名
```

改完至少跑一遍对应的 `cargo test` 与 `npm test`/`npm run build`，确认 `cargo check` 零警告。

## 发布一个版本

**必须遵守的发布约束：**

1. **每次发布必须带 changelog**。`release.yml` 会用 `scripts/release-changelog.mjs` 从上一个版本 tag 到本次 tag 的 Conventional commits 自动生成 changelog 写入 release body（`update-feed.mjs` 会把 release body 读进 feed 的 `selfUpdate.releaseNotes`，App 更新抽屉展示它）。因此提交信息要遵循 Conventional commits 前缀（`feat` / `fix` / `perf` / `refactor` / `docs` / `chore` / `ci` 等），不要发「无 changelog」的 release。手动触发 `release.yml`（workflow_dispatch）时也会生成 changelog。
2. **时序：必须先等 UManager 的 release 发布完成，再触发 update-feed**。`release.yml` 在发布 release 成功后会自动 `gh workflow run update-feed.yml`；不要手动提前跑 update-feed（它需要读取新 release 的资产摘要与 body）。

步骤：

1. 同步 bump 三处版本：`package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`；
2. `cargo check --manifest-path src-tauri/Cargo.toml`（同步 `Cargo.lock`）；
3. commit（按 Conventional commits 写提交信息）→ `git tag -a vX.Y.Z -m "UManager vX.Y.Z"` → `git push origin main` → `git push origin vX.Y.Z`；
4. `release.yml` 自动构建 `.deb`、用自动生成的 changelog 发布 release，并在 release 完成后自动触发 `update-feed` 让 feed 的 `selfUpdate` 刷新到新版本（带 releaseNotes）。

## 目录与数据流（一句版）

`feed-sources.json` + `vendors.json` →（CI `update-feed`）→ 签名 `feed.json` →（App `feed.rs` 拉取验签）→ 合并 `catalogJson` 新增软件 →（`source_engine`）`DownloadPlan` → 下载校验 →（`operation_plan`）不可变计划 →（特权 `helper`）验签 + 复核 → 固定 `dpkg`。
