# UManager

UManager 是面向 Ubuntu 的个人软件管家，聚焦从厂商官网安装的 `.deb` 应用，以及这些应用配置的厂商官方 APT 仓库。

当前版本已支持软件扫描、官方 APT 仓库安全更新、用户本地 `.deb` 安装和白名单软件卸载：

- 读取 `dpkg-query` 中已安装且被内置软件源明确支持的软件；
- 读取本机 `apt-cache policy`，识别候选版本和官方仓库来源；
- 使用 Debian 自身的版本比较规则判断是否有更新；
- VS Code、Google Chrome 和 ChatGPT Desktop 可从固定白名单中的官方 APT 仓库下载候选版本，实时展示进度与下载速度，校验并生成不可变计划；只有用户完成确认、特权 dry-run 和二次授权后才执行更新。
- “软件商店”页支持对 VS Code、Google Chrome、ChatGPT Desktop、微信和 FlClash 的新安装：未安装时从官方源锁定版本、大小与 SHA-256（微信为下载后计算），生成 `installedVersion: null` 的不可变计划。
- FlClash 通过 GitHub Releases API 读取最新稳定发布，以 GitHub 资产 SHA-256 与 HTTP Range 读出的 `.deb` 控制归档锁定版本，并走完整的下载校验、不可变计划、特权 dry-run 和安装链路。
- 受管软件卸载使用独立不可变计划、特权 dry-run 和二次授权，仅执行白名单中固定的 `dpkg --remove` 动作。
- 设置页会核对当前可执行文件与 `dpkg` 安装清单，区分 `.deb` 安装版、便携版和开发版；`.deb` 安装版可通过独立的 `remove-umanager` 计划安全卸载自身。
- 最终安装、更新和卸载会显示结构化进度与可展开的实时 `dpkg` 详细日志；日志只读，终端控制序列会被清理，单行与总量均有限制。
- “开发环境”页通过用户级版本管理器（nvm 用于 Node.js，rustup 用于 Rust）检测、安装、切换和卸载运行时版本，全程无 root。

## 开发环境

当前开发环境固定为：

- Ubuntu 22.04 或更新版本（当前应用目标为 amd64）；
- Node.js 24.19.0 LTS，版本记录在 `.nvmrc`；
- npm 11 或 12，项目使用 `package-lock.json`，不混用 pnpm；
- Rust 1.97 或更新的 stable 工具链，最低版本同时记录在各个 `Cargo.toml`；
- Tauri 2 所需的 WebKitGTK 和 Linux 编译库。

先安装 Ubuntu 原生依赖：

```bash
sudo apt update
sudo apt install \
  libwebkit2gtk-4.1-dev \
  build-essential \
  curl \
  wget \
  file \
  libxdo-dev \
  libssl-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev
```

Rust 建议通过 [rustup](https://rustup.rs/) 安装。进入项目目录后，Rust 工具链会根据 `rust-toolchain.toml` 自动选择；Node.js 可以通过 nvm 使用项目指定版本：

```bash
nvm install
nvm use
node --version
npm --version
rustc --version
```

本项目统一使用 npm。npm 随 Node.js 提供，无需再安装 pnpm；只提交 `package-lock.json`，避免不同包管理器解析出不同的依赖树。

## 启动开发环境

```bash
npm ci
npm run tauri dev
```

测试与构建：

```bash
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path crates/umanager-plan/Cargo.toml
cargo test --manifest-path crates/umanager-helper/Cargo.toml
```

软件源位于 `src-tauri/resources/vendors.json`。它被同时编译进主程序和特权 helper，是 UManager 认识哪些软件、从哪里下载、允许哪些域名的唯一事实来源。

## 软件源（vendors.json）

UManager 不再为每个软件写死适配器。新增一个软件只需要在 `src-tauri/resources/vendors.json` 的 `applications` 里追加一条记录：

- `applicationId` / `packageName` / `displayName` / `vendor` / `architecture`：应用标识与展示信息；
- `homepage` / `icon` / `accentColor`：可选的首页、图标键与强调色；
- `removable`：是否允许从 UManager 卸载；
- `source`：下载策略，按 `kind` 分为四类：
  - `aptRepository`：从官方 APT 仓库索引解析候选版本、大小与 SHA-256（如 VS Code、Google Chrome、ChatGPT Desktop）；
  - `stableDownloadEndpoint`：从官网固定地址下载，并从官网页面解析展示版本（如微信）；
  - `releaseApi`：从 GitHub Releases API 选择匹配资产，读取 `sha256:` 摘要（如 FlClash）；
  - `browserImport`：仅登记本机识别与卸载，不在软件商店提供自动下载（如腾讯会议）。

`source` 中声明的所有 `*Hosts` 都是精确域名白名单：下载、HTTP 重定向和 `apt-cache policy` 匹配都只接受这些域名，拒绝相似域名。主程序的通用下载引擎（`src-tauri/src/source_engine.rs`）与 helper 的白名单校验都从同一份内置 JSON 生成，因此新增软件不需要改动 Rust 代码，也不需要新增 Tauri command。

## 开发环境（用户级版本管理器）

除了 `.deb` 桌面软件，UManager 也支持通过**用户级版本管理器**安装的开发工具。这类工具不经过 dpkg、不需要 root，因此走一条独立的无特权链路（`src-tauri/src/dev_tools.rs`）。

在 `vendors.json` 的 `developmentToolchains` 数组里登记一条记录即可接入：

- `toolchainId` / `displayName` / `vendor` / `homepage`：工具链标识与展示信息；
- `manager`：版本管理器标识，如 `nvm`、`rustup`；
- `managerKind`：`shell`（管理器是 shell 函数，需 source 脚本）或 `binary`（管理器是磁盘上的可执行文件）；
- `managerHome`：管理器数据目录（如 `~/.nvm`、`~/.rustup`）；`managerScript` 记录 shell 型管理器要 source 的脚本；`managerBinary` 记录 binary 型管理器的可执行路径；
- `versionsDirectory`：存放各版本子目录的路径。

当前内置两条工具链：
- **Node.js（nvm，shell）**，`managerKind: shell`，通过 `~/.nvm/nvm.sh` 调用；
- **Rust（rustup，binary）**，`managerKind: binary`，直接调用 `~/.cargo/bin/rustup`。

“开发环境”页会：

1. 检测管理器（优先尊重 `NVM_DIR` / `RUSTUP_HOME`），读取管理器版本、默认版本和已安装版本；
2. 读取可安装版本：Node.js 走 `nvm ls-remote --lts`，Rust 提供 `stable / beta / nightly` 通道；
3. 支持“安装并设为默认”、“设为默认”和“卸载”，实时展示版本管理器输出。

nvm 是 shell 函数，因此通过 `/bin/bash -c 'source nvm.sh --no-use; nvm …'` 执行；rustup 是可执行文件，直接以参数向量调用。两者都只由固定子命令 + 经过严格校验的版本/别名 token 组成，不接收任意 shell。该链路始终以当前用户身份运行，不经过 Polkit helper，也不请求 root。

## Visual Studio Code 适配

VS Code 现已具备完整的官方 APT 适配器：

- 验证 Debian 包名为 `code`、架构为 `amd64`；
- 精确匹配微软官方仓库域名 `packages.microsoft.com`，拒绝相似域名；
- 优先采用官方 APT 仓库和候选版本；
- 未发现可信仓库时，规划回退到官方 stable 下载接口；
- 展示后续下载需要执行的域名、包名、架构、版本与 SHA-256 校验计划。

点击软件列表中的 Visual Studio Code 可查看完整证据与只读操作计划。

VS Code 详情页还可以从已验证的 APT 索引生成下载计划。存在更新时，UManager 会使用 Reqwest/Rustls 流式下载到应用缓存中的唯一临时文件，并依次校验官方域名、响应大小、SHA-256、Debian 包名、版本和架构。全部通过后才以不覆盖现有文件的方式写入缓存；此过程不安装软件，也不请求 root 权限。

## 微信适配

微信使用官网服务器渲染的三段版本号与固定 x86_64 `.deb` 下载地址。UManager 校验 `linux.weixin.qq.com` 和 `dldir1v6.qq.com` 精确域名，通过 HTTP Range 先读取 4 KiB，解析 Debian ar 目录并精确截取 `control.tar.*` 控制归档；若控制区超出探测片段，再按解析出的准确长度补取，且硬性限制在 4 MiB 内。随后一次性获取包名 `wechat`、完整版本和 `amd64` 架构。官网与控制区并行请求，远端元数据在进程内缓存 15 分钟，本机状态只查询 `wechat` 单包。因此即使网页只显示如 `4.1.1` 的三段版本，仍会以 `.deb` 中的完整版本（如 `4.1.1.8`）执行 Debian 版本比较。检查过程不下载完整安装包。

微信检查已纳入主页面“检查更新”，可信通道显示为“官网直连”而不是“本地 `.deb`”。发现严格更高版本后，详情抽屉可从固定官网地址流式下载完整安装包，实时显示进度和速度，并复核响应大小、包名 `wechat`、完整版本、网页版本前缀和 `amd64` 架构。下载后计算 SHA-256 并锁入 `InstallVerifiedWebsiteDeb` 不可变计划；helper 只接受固定的 `install-verified-website-deb` 动作，重新核对当前已安装版本、架构、缓存路径、大小、SHA-256 和包元数据后，才以固定 `dpkg --install` 执行。

微信官网目前未发布独立的签名 SHA-256 清单，因此该哈希用于保证“下载完成后到安装前文件未变化”，不能提供与签名 APT 索引相同的发布者哈希证明。UI 会在用户确认前明确展示这一信任边界。

## FlClash 适配

FlClash 通过 `https://api.github.com/repos/chen08209/FlClash/releases/latest` 读取最新稳定发布，校验精确域名 `api.github.com`，并在资产列表中精确匹配 `FlClash-<tag>-linux-amd64.deb`；发布资产同时提供大小与 `sha256:` 摘要。下载地址必须位于 `github.com`，实际读取时允许重定向到 GitHub 的 `objects.githubusercontent.com` 与 `release-assets.githubusercontent.com`。

与微信相同，UManager 用 HTTP Range 先读取 4 KiB，解析 Debian ar 目录并精确截取 `control.tar.*` 控制归档，再以 `dpkg-deb --field` 单字段读取包名、版本和架构。由于 FlClash 控制文件中的 `Package` 为大写，适配器使用 `dpkg-deb --field <path> Package` 让包名按 Debian 规范归一化为小写 `flclash`，并以完整版本（如 `0.8.96+2026081701`）执行 Debian 版本比较。远端元数据在进程内缓存 15 分钟，检查过程不下载完整安装包。

FlClash 检查已纳入主页面“检查更新”，可信通道显示为“官网直连”。发现严格更高版本后，详情抽屉可从 GitHub 发布资产流式下载完整安装包，实时显示进度和速度，并复核响应大小、GitHub 资产 SHA-256、包名 `flclash`、完整版本与 `amd64` 架构。下载后哈希与包元数据锁入 `InstallVerifiedWebsiteDeb` 不可变计划；helper 重新核对当前已安装版本、架构、缓存路径、大小、SHA-256 和包元数据后，才以固定 `dpkg --install` 执行。

GitHub Releases 资产摘要由 GitHub 在发布时生成，比微信官网多一层发布者哈希，但仍不是签名 APT 索引级别的证明。UI 会在用户确认前明确展示这一信任边界。

## 本地 `.deb` 安装

Linux `.deb` 包会关联到 UManager。用户在文件管理器中选择“使用 UManager 打开”后，应用会：

1. 无特权读取包名、版本和架构，计算大小与 SHA-256；
2. 明确标记为“用户提供，来源未验证”，拒绝同版本重装、降级和不兼容架构；
3. 以不覆盖方式复制到 UManager `imports` 缓存，并重新校验；
4. 在用户明确确认后生成 15 分钟内有效的不可变计划；
5. 通过 Polkit helper 先执行 dry-run 复核，只有用户再次点击安装后才以固定 `/usr/bin/dpkg --install <root-owned-staged-deb>` 参数执行。

本地 `.deb` 可包含以 root 权限运行的维护脚本，UManager 无法为用户提供的任意包证明发布者身份，因此 UI 会在授权前显示这一安全边界。

## 新安装

“软件商店”页会逐一检查五个受管应用的 dpkg 状态与官方源：VS Code、Google Chrome 和 ChatGPT Desktop 走官方 APT 仓库索引，微信走官网固定地址，FlClash 走 GitHub Releases 资产。只有应用未安装、候选版本存在、架构为 amd64 且能拿到大小与 SHA-256（微信下载后计算）时，UI 才开放下载。

下载后会复核 HTTPS 精确域名、重定向、文件大小、SHA-256 以及 `.deb` 中的包名、版本和架构。特权 helper 在 dry-run 和真正安装前都会再次确认应用仍未安装、当前系统为 amd64，并将计划与官方源记录重新对比，最后只以固定 `/usr/bin/dpkg --install <root-owned-staged-deb>` 参数执行。

## 卸载

UManager 列表中的六个白名单软件可以卸载。用户核对包名、已安装版本和架构后，应用会生成 15 分钟内有效的独立不可变卸载计划。helper 先在 dry-run 中重新核对计划、白名单和当前 dpkg 状态，只有用户再次授权后才以固定 `/usr/bin/dpkg --remove <package>` 参数执行。

卸载不请求 `purge`，不自动移除依赖，也不由 UManager 直接删除用户主目录数据。Debian 包自带的维护脚本仍会以 root 权限运行，反向依赖不允许卸载时会原样报错，UManager 不使用强制参数绕过。

UManager 自身的卸载入口位于“设置”。只有当前运行的可执行文件确实包含在已安装的 `u-manager` Debian 包清单中时，入口才会启用；开发版和便携版不会因为系统中恰好存在另一个 UManager 包而误判。自卸载使用单独的不可变计划和 helper 动作，执行前重新核对固定包名、版本和架构，最终仅调用 `/usr/bin/dpkg --remove u-manager`。缓存、下载文件、操作计划和个人数据会保留。

下载校验后，用户可核对最终计划并显式确认。计划以 payload 的 SHA-256 作为 ID，以不覆盖方式写入只读文件，有效期最长 15 分钟。独立 `umanager-helper` 只接受固定动作：`aptRepository` 类应用允许 `install-verified-deb ... --dry-run` 和 `--execute`；`stableDownloadEndpoint` 与 `releaseApi` 类应用允许 `install-verified-website-deb ... --dry-run` 和 `--execute`；用户本地包允许 `install-local-deb ... --dry-run` 和 `--execute`；`vendors.json` 中 `removable: true` 的应用允许 `remove-managed-package ... --dry-run` 和 `--execute`；UManager 自身只允许 `remove-umanager ... --dry-run` 和 `--execute`。helper 的这三张白名单同样从内置 JSON 生成，并会重新检查计划完整性、有效期、白名单、当前安装状态，以及安装包的缓存路径、文件归属、元数据、官方仓库记录、大小和 SHA-256，且不经过 shell。

## 安全边界

React 只调用类型明确的 Tauri commands。Rust 端仅以固定参数调用 `dpkg-query`、`apt-cache`、`dpkg-deb` 和 `dpkg --compare-versions`，不经过 shell，也不接受前端传入的命令或包名。`.deb` 打包配置会将 helper 安装到 `/usr/libexec/umanager-helper`，并将 Polkit policy 安装到 `/usr/share/polkit-1/actions/`。
