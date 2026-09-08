# UManager

> 面向 Ubuntu 的个人软件管家：扫描、安装、更新、卸载来自厂商官网与官方仓库的 `.deb` 应用，每一步都经过签名校验与特权复核。

UManager 是基于 [Tauri 2](https://tauri.app/)（Rust 后端 + React 前端）的桌面应用。软件信息只来自签名元数据源，系统级操作全部经 Polkit 特权 helper 执行，遵循「不可变计划 + 特权 dry-run + 再次确认」的安全模型。

## 它能做什么

**软件管理**

- 扫描本机已安装的受管应用，区分 `.deb` 安装版 / 便携版 / 开发版；
- 从厂商官网或官方仓库安装、更新、卸载 `.deb` 应用，按「全部 / 已安装 / 可更新」三个 Tab 浏览；
- 下载时核对 HTTPS 精确域名、响应大小、SHA-256，以及 `.deb` 的包名 / 版本 / 架构；
- 双击本地 `.deb` 即可用 UManager 打开安装；
- 从 GitHub Release 校验并一键升级 UManager 自身。

**开发环境**

- 运行时工具链：Node.js（nvm）与 Rust（rustup）的检测、安装、切换、卸载，全程无 root；
- CLI AI 编程工具：Claude Code、OpenCode、Pi、Codex CLI、DeepSeek Harness、Hermes Agent，识别 npm / 官方安装器 / PATH 来源并安装、更新、卸载。

**安全模型**

- 软件信息只来自 Ed25519 签名的 feed，不在本机抓取官网或解析 APT 索引；
- 特权动作走固定白名单、固定 argv 的 Polkit helper，不经过 shell；
- 每次操作先 dry-run 复核，用户再次确认才执行；同一登录会话只弹一次管理员授权。

## 支持的应用

| 类别 | 应用 |
|---|---|
| 开发工具 | Visual Studio Code、GitHub CLI、Wine（Windows 兼容层） |
| 浏览器 | Google Chrome、Microsoft Edge |
| 聊天协作 | 微信、QQ、飞书、ChatGPT Desktop、钉钉（仅识别 / 卸载） |
| 办公效率 | WPS Office、腾讯会议、腾讯文档、Obsidian、Bitwarden |
| 网络 | FlClash、LocalSend |
| 影音娱乐 | QQ 音乐 |

> 应用清单由内置的 `vendors.json` 与 CI 签发的 feed 共同决定；新增应用无需升级 UManager。

## 安装

面向 Ubuntu 22.04 及以上（amd64）。普通用户**无需克隆源码**，也不需要安装 Node.js 或 Rust。

1. 打开 [Releases](https://github.com/null-object-0000/umanager/releases) 页面；
2. 下载最新的 `UManager_<版本>_amd64.deb`；
3. 在下载目录执行：

```bash
sudo apt install ./UManager_*_amd64.deb
```

`apt` 会自动补装运行时依赖（`libwebkit2gtk-4.1-0`、`libgtk-3-0`）。

也可以直接命令行下载并安装（以 v0.1.0 为例，升级时替换版本号）：

```bash
wget https://github.com/null-object-0000/umanager/releases/download/v0.1.0/UManager_0.1.0_amd64.deb
sudo apt install ./UManager_0.1.0_amd64.deb
```

安装后从应用菜单搜索 “UManager” 启动，或在终端运行 `umanager`。卸载入口在应用“设置”页，或执行 `sudo apt remove u-manager`。

## 开发

面向贡献者。详细架构、数据流与关键不变量见 [ARCHITECTURE.md](ARCHITECTURE.md)；给 AI 编程 agent 的工作指引见 [AGENTS.md](AGENTS.md)。

**环境要求**

- Ubuntu 22.04 或更新版本（amd64）；
- Node.js 24.19.0 LTS（版本记录在 `.nvmrc`）+ npm 11/12（项目使用 `package-lock.json`，不混用 pnpm）；
- Rust stable 工具链（`rust-toolchain.toml` 自动选择）；
- Tauri 2 所需的 WebKitGTK 与 Linux 编译库。

**快速开始**

```bash
# 系统依赖
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev

# 运行时（Node 用 nvm，Rust 用 rustup）
nvm install && nvm use

# 启动开发环境
npm ci
npm run tauri dev
```

**测试与构建**

```bash
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path crates/umanager-plan/Cargo.toml
cargo test --manifest-path crates/umanager-helper/Cargo.toml
```

## 文档

| 文档 | 面向 |
|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | 架构、feed、数据流、安全不变量、新增应用方式 |
| [AGENTS.md](AGENTS.md) | AI 编程 agent 与协作者的开发指引 |

## Windows 应用（Wine）

在「Windows 应用」页管理企业微信 Windows 版：检测已有安装、启动、安装、更新、卸载，以及修改专用 Wine 环境的配置。

- 首次使用前准备 Wine 和 `fonts-noto-cjk`；Wine 可从 UManager 软件商店管理。
- 自动沿用 `~/.local/share/wineprefixes/wecom`（若存在），否则在 `~/.local/share/umanager/windows/wecom` 创建独立环境。
- 默认配置为 Windows 10、192 DPI、X11/XWayland、Tahoma 的 Noto Sans CJK SC 字体替换，以及标题栏修复。设置可在页面修改，不复制账号、聊天记录或整份注册表。
- 安装/更新从签名 feed 获取官方 EXE 地址、版本、大小及 SHA-256；下载校验后复核确认，再打开官方向导。保持默认安装目录，完成后退出企业微信，UManager 检查实际安装版本。
- 卸载运行官方卸载向导；聊天记录的保留选项由用户在向导中选择。UManager 保留 Wine 环境目录，不递归删除已有环境。
- 安装、更新、卸载和配置均以普通用户运行，不使用 Polkit。应用运行时需先从托盘退出。
- 新安装和更新需要 CI 发布带 `windowsApplications` 的签名 feed；旧 feed 下仍可管理本机安装，但不会降级到本地抓取或执行未校验的安装包。

开发构建额外需要 `gcc-mingw-w64-x86-64`，用于从源码编译随 App 内置的 Windows 标题栏辅助程序；可通过 `UMANAGER_MINGW_CC` 指定交叉编译器。最终用户不需要编译器。

Wine 生命周期集成测试使用隔离显示和一次性环境中的专用测试 EXE，覆盖安装、更新、配置、卸载，不操作真实企业微信：

```bash
xvfb-run -a env WINEDLLOVERRIDES=winemenubuilder.exe=d \
  cargo test --manifest-path src-tauri/Cargo.toml \
  wine_install_update_configure_uninstall_lifecycle -- --ignored --nocapture
```

这项测试验证管理流程；企业微信官方安装向导，以及聊天、音视频、文件传输兼容性仍需真实应用验收。
