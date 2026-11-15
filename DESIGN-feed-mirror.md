# UManager 签名更新数据源镜像

镜像入口为 `https://updates.snewbie.site/umanager/`，使用 EdgeOne 全球可用区（含中国大陆）。GitHub Pages 的发布流程与地址保持原样。

## 发布链路

原 `update-feed.yml` 继续完成抓取、签名与 GitHub Pages 发布；生成器、签名密钥与 release → update-feed 时序不变。Pages 发布成功后，独立的轻量 job 显式 dispatch 镜像工作流，仅该 job 具有 Actions 写权限，不接触签名或腾讯云 Secrets。它失败不会使规范 Pages 发布失效。

独立的 `mirror-feed.yml` 在 main 分支 Pages 发布成功后由上述 job 启动，也支持在 main 上手动运行。显式 dispatch 避免 release 用 `GITHUB_TOKEN` 启动 update-feed 后，后续 `workflow_run` 事件没有继续触发的问题。它在执行时选择最近一次 GitHub Pages 发布成功的规范产物：即使某个抓取组失败，只要 merge 用旧数据补齐后成功发布，也会同步；整个工作流尚在结束 dispatch job 时也可使用已完成的 merge。下载该次 Pages 的固定 artifact，验证后原样部署；若最新已发布 artifact 过期则失败，不静默降级成更旧快照。镜像失败不会阻止 GitHub 发布。部署任务只有读取 Actions artifact 的权限与腾讯云部署 Secrets，不持有任何 `FEED_SIGNING_KEY*`。

`scripts/edgeone/extract-pages-artifact.py` 拒绝归档穿越、链接、特殊文件、重复路径和异常大小。`prepare.mjs` 从客户端现有常量读取中央公钥，验证 v2/v3 feed、catalog、中央源背书和图标摘要；只有验证全部通过才复制原始字节。镜像不重新序列化或重新签名任何 feed。`mirror-status.json` 仅用于运维核对，不是客户端信任来源。

腾讯云 Secrets 为 `TENCENT_CLOUD_SECRET_ID`、`TENCENT_CLOUD_SECRET_KEY`；部署工具依赖独立锁定在 `scripts/edgeone`。发布目录没有 SPA fallback，缺失文件应返回失败，不能伪装为有效 feed。

## 客户端与信任边界

`feed_transport.rs` 只为原精确白名单已允许的 `https://null-object-0000.github.io/umanager/` 路径添加固定镜像候选。它不改变配置中的规范 URL，也不重写签名数据里的源 URL、公钥或背书。任意自定义域名、其他 GitHub 项目、非 HTTPS、额外端口、凭据、查询和片段不自动套用镜像。

每个候选单独获取 feed 与对应 `.sig`，并通过原 `fetch_feed_at` / `verify_ed25519_with` 后才算成功。镜像失败时整组回退，不拼接镜像 JSON 与 GitHub 签名。图标同样先通过原大小、HTTPS、域名和 SHA-256 检查，再写入缓存。安装包下载路径与白名单、特权 helper、feed/plan schema 和缓存身份不变。

自 v0.18.4 起，客户端优先使用镜像；旧安装版需要升级到 v0.18.4 或更新版本后才会启用此传输策略。升级不改变签名公钥、验签逻辑、规范源身份或 GitHub 回退。

## UManager 安装包镜像（v0.18.5 起）

仅 UManager 自身的 amd64 Debian 包支持固定传输镜像，路径为 `https://updates.snewbie.site/umanager/releases/v版本/UManager_版本_amd64.deb`。签名 feed 与下载计划仍声明 GitHub Release 原始 URL，`vendors.json`、helper 授权、公钥、签名、其他软件的下载白名单不变。只有应用 ID、包名、架构、三段数字版本、GitHub 仓库路径和文件名全部精确匹配时才生成镜像候选；镜像只允许自身精确主机，GitHub 回退使用原有主机白名单。

后台下载队列对每个候选执行原有 HTTPS、重定向白名单、响应及实际大小、SHA256、Debian 包名/版本/架构检查。镜像限时 60 秒；网络、404 或完整校验失败均先删除临时文件再从 GitHub 重新下载。用户取消不会触发回退。校验完成后才进入原有缓存与安装路径。

镜像 CI 在已验证的 Pages 快照旁运行 `scripts/edgeone/package.mjs`，重新核对中央 feed 签名，从精确 GitHub Release 地址获取当前包，并验证签名 feed 的大小和 SHA256 后原样发布。只保留当前快照对应的包；旧版本路径可能返回 404，客户端自动回退 GitHub。免费单文件上限按 25 MiB 控制，超限或包同步失败不阻止元数据镜像发布，客户端继续走 GitHub。`package-status.json` 是运维信息，不参与客户端信任。

## 验证

- `npm test --prefix scripts/edgeone`：字节保持、签名错误、错误源密钥、背书和图标摘要拒绝。
- `python3 -m unittest discover -s scripts/edgeone/test -p 'test_*.py'`：归档边界。
- `cargo test --manifest-path src-tauri/Cargo.toml --lib feed`：客户端验签契约、精确域名范围及已验证的整体回退。
- `test-feed-mirror.yml` 在 Ubuntu 上运行以上检查、`cargo check`、前端测试和构建。
- 上线后按 `mirror-status.json` 对照原 artifact 的 SHA-256，并用客户端原公钥核对公开镜像的全部 feed 签名。
