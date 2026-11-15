# UManager 开发指南

[返回项目首页](../README.md)

## 开发

面向贡献者。详细架构、数据流与关键不变量见 [ARCHITECTURE.md](../ARCHITECTURE.md)；给 AI 编程 agent 的工作指引见 [AGENTS.md](../AGENTS.md)。

**环境要求**

- Ubuntu 22.04 或更新版本（amd64）；
- Node.js 24.19.0 LTS（版本记录在 `.nvmrc`）+ npm 11/12（项目使用 `package-lock.json`，不混用 pnpm）；
- Rust stable 工具链（`rust-toolchain.toml` 自动选择）；
- Tauri 2 所需的 WebKitGTK 与 Linux 编译库。

**快速开始**

```bash
# 系统依赖
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libssl-dev libayatana-appindicator3-dev librsvg2-dev

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


构建官网：`node website/build.mjs`。官网为独立静态页面，不需要编译桌面客户端；下载版本与摘要从现有公钥验证通过的 feed 生成。官网在相关源码合入 main 后部署；签名数据与安装包镜像发布成功后也会调用同一部署工作流，保持下载入口更新。
