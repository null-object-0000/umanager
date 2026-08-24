# UManager

UManager 是面向 Ubuntu 的个人软件管家，聚焦从厂商官网安装的 `.deb` 应用，以及这些应用配置的厂商官方 APT 仓库。

当前版本已支持只读软件扫描、厂商更新 dry-run 和用户本地 `.deb` 安装：

- 读取 `dpkg-query` 中已安装且被厂商目录明确支持的软件；
- 读取本机 `apt-cache policy`，识别候选版本和官方仓库来源；
- 使用 Debian 自身的版本比较规则判断是否有更新；
- 厂商自动更新仍只执行特权 dry-run；用户明确选择的本地 `.deb` 在完成两阶段确认与特权复核后可执行安装。

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

厂商适配目录位于 `src-tauri/resources/vendors.toml`。首个计划完整打通的适配器是 Visual Studio Code。

## Visual Studio Code 适配

VS Code 现已具备完整的只读适配器：

- 验证 Debian 包名为 `code`、架构为 `amd64`；
- 精确匹配微软官方仓库域名 `packages.microsoft.com`，拒绝相似域名；
- 优先采用官方 APT 仓库和候选版本；
- 未发现可信仓库时，规划回退到官方 stable 下载接口；
- 展示后续下载需要执行的域名、包名、架构、版本与 SHA-256 校验计划。

点击软件列表中的 Visual Studio Code 可查看完整证据与只读操作计划。

VS Code 详情页还可以从已验证的 APT 索引生成下载计划。存在更新时，UManager 会使用 Reqwest/Rustls 流式下载到应用缓存中的唯一临时文件，并依次校验官方域名、响应大小、SHA-256、Debian 包名、版本和架构。全部通过后才以不覆盖现有文件的方式写入缓存；此过程不安装软件，也不请求 root 权限。

## 微信适配

微信使用官网服务器渲染的三段版本号与固定 x86_64 `.deb` 下载地址。UManager 校验 `linux.weixin.qq.com` 和 `dldir1v6.qq.com` 精确域名，再通过 HTTP Range 只读取安装包前 8 MiB，从 Debian 控制归档中获取包名 `wechat`、完整版本和 `amd64` 架构。因此即使网页只显示如 `4.1.1` 的三段版本，仍会以 `.deb` 中的完整版本（如 `4.1.1.8`）执行 Debian 版本比较。检查过程不下载完整安装包。

## 本地 `.deb` 安装

Linux `.deb` 包会关联到 UManager。用户在文件管理器中选择“使用 UManager 打开”后，应用会：

1. 无特权读取包名、版本和架构，计算大小与 SHA-256；
2. 明确标记为“用户提供，来源未验证”，拒绝同版本重装、降级和不兼容架构；
3. 以不覆盖方式复制到 UManager `imports` 缓存，并重新校验；
4. 在用户明确确认后生成 15 分钟内有效的不可变计划；
5. 通过 Polkit helper 先执行 dry-run 复核，只有用户再次点击安装后才以固定 `/usr/bin/dpkg --install <root-owned-staged-deb>` 参数执行。

本地 `.deb` 可包含以 root 权限运行的维护脚本，UManager 无法为用户提供的任意包证明发布者身份，因此 UI 会在授权前显示这一安全边界。

下载校验后，用户可核对最终计划并显式确认。计划以 payload 的 SHA-256 作为 ID，以不覆盖方式写入只读文件，有效期最长 15 分钟。独立 `umanager-helper` 只接受固定动作：官方 VS Code 计划只允许 `install-verified-deb ... --dry-run`；用户本地包允许 `install-local-deb ... --dry-run` 和 `--execute`。helper 会重新检查计划完整性、有效期、缓存路径、文件归属、包名、版本、架构、大小和 SHA-256，且不经过 shell。

## 安全边界

React 只调用类型明确的 Tauri commands。Rust 端仅以固定参数调用 `dpkg-query`、`apt-cache`、`dpkg-deb` 和 `dpkg --compare-versions`，不经过 shell，也不接受前端传入的命令或包名。`.deb` 打包配置会将 helper 安装到 `/usr/libexec/umanager-helper`，并将 Polkit policy 安装到 `/usr/share/polkit-1/actions/`。
