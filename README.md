# UManager

> 面向 Ubuntu 的个人软件管家：扫描、安装、更新、卸载来自厂商官网与官方仓库的 `.deb` 应用，每一步都经过签名校验与特权复核。

UManager 是基于 [Tauri 2](https://tauri.app/)（Rust 后端 + React 前端）的桌面应用。软件信息只来自签名元数据源，系统级操作全部经 Polkit 特权 helper 执行，遵循「不可变计划 + 特权 dry-run + 再次确认」的安全模型。

## 它能做什么

**软件管理**

- 扫描本机已安装的受管应用，区分 `.deb` 安装版 / 便携版 / 开发版；
- 从厂商官网或官方仓库安装、更新、卸载 `.deb` 应用，按「全部 / 已安装 / 可更新」三个 Tab 浏览；
- 下载时核对 HTTPS 精确域名、响应大小、SHA-256，以及 `.deb` 的包名 / 版本 / 架构；
- 安装 / 更新分两阶段：安装包先进后台下载队列（可排队、可取消，并发数 1–3 可调），下载期间可以离开详情页去装别的软件或逛别的页面；下载校验完成后到「下载」页点「安装 / 更新」，才进入需要管理员授权的第二阶段；
- 双击本地 `.deb` 即可用 UManager 打开安装；
- 从 GitHub Release 校验并一键升级 UManager 自身。

**开发环境**

- 运行时工具链：Node.js（nvm）与 Rust（rustup）的检测、安装、切换、卸载，全程无 root；
- CLI AI 编程工具：Claude Code、OpenCode、Pi、Codex CLI、DeepSeek Harness、Hermes Agent，识别 npm / 官方安装器 / PATH 来源并安装、更新、卸载。

**固件更新**

- 「工具 → 固件更新」通过系统 `/usr/bin/fwupdmgr` 读取设备、当前版本、可用更新及更新说明，无需 Snap；
- 展示接电源、重启要求与设备限制，区分不支持、无可用更新和查询失败；
- 检查使用 fwupd 已缓存的元数据，不自动刷新或安装；缓存无更新不等于厂商没有新版本。未安装 fwupd 时显示缺失提示；
- 按设备和目标版本创建 15 分钟有效的只读计划，核对后点击「授权并复核（不安装）」，管理员复核通过后再次勾选确认并更新。helper 执行前再次核对设备标识、当前版本、可信元数据、目标版本、来源与校验信息；状态变化时拒绝执行；
- 固件由系统 fwupd 下载、校验与写入，不允许强制、降级、重装或全部设备更新，不自动重启或关机。更新中请勿断电、拔出设备或强制退出；失败后先重新检查设备状态，避免直接重复刷写。

固件安装需要配套的新版本 `/usr/libexec/umanager-helper` 和 Polkit policy；只运行前端或沿用旧 helper 不能安装。浏览器开发预览中的授权和安装均为模拟，不访问真实硬件。

**安全模型**

- 软件信息只来自 Ed25519 签名的 feed，不在本机抓取官网或解析 APT 索引；
- 特权动作走固定白名单、固定 argv 的 Polkit helper，不经过 shell；
- 每次操作先 dry-run 复核，用户再次确认才执行；同一登录会话只弹一次管理员授权。

### 可选的彻底清理

卸载 `.deb` 应用时，默认仍保留个人数据、软件包配置及依赖。需要释放空间时，在卸载窗口开启「彻底清理」，点击「扫描残留并预览清单」，逐项勾选后锁定计划。管理员复核通过后，还需要再次确认才会开始删除。

清单显示个人目录 / 安装包缓存的路径和实际占用、系统残留文件，以及可移除依赖的包名和版本。依赖必须同时满足：位于目标软件的已安装依赖链中、标记为自动安装、APT 预演允许移除。手动安装、共享、Essential / Protected 软件包及其他无关孤立依赖不会作为候选项。取消部分依赖后若会破坏剩余软件，复核将拒绝该清单。

个人数据规则目前覆盖 Docker Desktop、VS Code、Chrome 稳定版和 Edge 稳定版；其他软件仅自动检测 UManager 中对应的 `.deb` 缓存，界面会提示未覆盖的范围。Docker 软件源 / 密钥只有在相关组件均不再保留时才允许删除。其余未知目录及软件源需另行检查，不按名称猜测删除。UManager 自卸载暂不提供清理自身数据的选项。

数据删除无法撤销，请先退出目标软件并核对清单。清理计划有效期为 15 分钟；执行前再次核对依赖版本与目录身份，状态变化时需要重新扫描。个人文件始终以当前用户权限清理；软件包及审核过的系统文件由 helper 处理。部分清理失败会列出未删除的项目。厂商包自带的卸载 / purge 脚本也会运行，它们可能自行处理配置文件。

## 支持的应用

| 类别 | 应用 |
|---|---|
| 开发工具 | Visual Studio Code、GitHub CLI、Wine（Windows 兼容层） |
| 浏览器 | Google Chrome、Microsoft Edge |
| 聊天协作 | 微信、QQ、飞书、ChatGPT Desktop、钉钉（仅识别 / 卸载） |
| 办公效率 | WPS Office、腾讯会议、腾讯文档、Obsidian、Bitwarden、FigCraft |
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
- 安装/更新从签名 feed 获取官方 EXE 地址、版本、大小及 SHA-256；点「更新」后先下载并校验安装包（卡片按钮会变成进度环，下载期间可以继续使用企业微信），校验完成后再复核确认、打开官方向导。保持默认安装目录；安装器可能顺带启动企业微信，不必刻意退出，UManager 以磁盘上的实际安装版本判定完成。
- 卸载运行官方卸载向导；聊天记录的保留选项由用户在向导中选择。UManager 保留 Wine 环境目录，不递归删除已有环境。
- 安装、更新、卸载和配置均以普通用户运行，不使用 Polkit。下载校验阶段无需退出企业微信，但真正安装、卸载或应用配置前必须先退出；详情页提供「强制停止」，按 prefix 结束该 Wine 环境里的企业微信进程（可能丢失未发送的输入）。
- 新安装和更新需要 CI 发布带 `windowsApplications` 的签名 feed；旧 feed 下仍可管理本机安装，但不会降级到本地抓取或执行未校验的安装包。

开发构建额外需要 `gcc-mingw-w64-x86-64`，用于从源码编译随 App 内置的 Windows 标题栏辅助程序；可通过 `UMANAGER_MINGW_CC` 指定交叉编译器。最终用户不需要编译器。

Wine 生命周期集成测试使用隔离显示和一次性环境中的专用测试 EXE，覆盖安装、更新、配置、卸载，不操作真实企业微信：

```bash
xvfb-run -a env WINEDLLOVERRIDES=winemenubuilder.exe=d \
  cargo test --manifest-path src-tauri/Cargo.toml \
  wine_install_update_configure_uninstall_lifecycle -- --ignored --nocapture
```

这项测试验证管理流程；企业微信官方安装向导，以及聊天、音视频、文件传输兼容性仍需真实应用验收。
