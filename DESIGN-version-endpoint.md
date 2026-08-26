# DESIGN：v0.3.0 功能范围（`versionEndpoint` + 图标远端化 + 软件描述）

> 状态：**设计草案，待评审。未改任何代码。**
> 目标：让 UManager 支持「没有 APT 仓库、没有 GitHub Release、没有固定 .deb 直链，只有官网动态下发版本与下载地址」的软件——QQ、QQ 音乐、飞书、腾讯会议等；同时把「新增软件**免发版**」贯彻到位：软件版本、图标、描述都随签名 feed 下发，App 拉取缓存。

## 0. v0.3.0 整合范围（一次发版全部落地）

| # | 功能 | 关键改动 | 是否触发放版 |
|---|---|---|---|
| 1 | 新增 `versionEndpoint` source kind | catalog 枚举 + 生成器 + （QQ 的）App 签名子步骤 | ✅ |
| 2 | 新增软件：QQ / QQ音乐 / 腾讯会议 / Obsidian / Bitwarden | `feed-sources.json`（+ 腾讯会议由 `browserImport` 升级） + 图标 | ✅（其实 feed 条目免发版，kind/图标/展示走发版） |
| 3 | 已免发版：GitHub CLI、LocalSend | `feed-sources.json`（`aptRepository` / `releaseApi`），无 App 改动 | ❌ |
| 4 | 图标远端化 | CI 从 .deb 提取图标发 Pages + feed `iconUrl/iconSha256` + App 命令/前端缓存 | ✅ |
| 5 | 软件/工具文字描述 | catalog `description` 字段 + 前端展示 | ✅ |
| 6 | 「系统级 CLI 工具」区块 | 开发环境第三区块，复用 .deb 特权链路（gh 归位） | ✅ |
| 7 | `stableDownloadEndpoint.page_version_marker` 可选化 | 支持 Bitwarden 这类固定最新 URL | ✅ |

> 说明：feeds 条目（3）本身免发版，但图标（4）与描述（5）需要 App 认识新字段，故与 `versionEndpoint` 一起排进 v0.3.0。

## 1. 背景与问题

UManager 的软件信息只来自 CI 抓取并 Ed25519 签名的 feed（`feed.json` + `catalogJson`）。现有三种「可自动安装」的 source kind：

| kind | 适合 | 不满足的场景 |
|---|---|---|
| `aptRepository` | 官方 APT 仓库（VS Code / Chrome / ChatGPT） | 无 Packages 索引 |
| `releaseApi` | GitHub Releases（FlClash / 自更新） | 无 GitHub 仓库 |
| `stableDownloadEndpoint` | **固定** .deb 直链 + 官网 HTML 版本标记（微信） | 下载地址随版本变化 |

QQ / QQ 音乐 / 飞书 / 腾讯会议四者都落在「官网动态下发版本 + 下载地址」这一类，现有三种 kind 都套不上。因此需要新增一种 source kind。（网易云音乐官方已无 Linux 端，不在范围内。）

**Telegram 不在本次范围**：官方 Linux 客户端只发 tar.xz / snap / flatpak，没有官方 `.deb`，与 UManager「只管理 `.deb`」的定位不符（已确认跳过）。

## 2. 调研结论（截至本草案）

| 软件 | packageName | 版本/下载来源 | 结论 |
|---|---|---|---|
| **QQ** | `linuxqq` | 新官网 `im.qq.com/index/#/linux` 拉取**纯 JSON** 配置 `https://qq-web.cdn-go.cn/im.qq.com_new/latest/rainbow/pcConfig.json`（`Linux.version` + `Linux.x64DownloadUrl.deb`） | ✅ 数据源已用浏览器实测；⚠️ 原始 .deb 地址 403，**必须先调签名接口换签名地址才能下载**（签名接口无鉴权、境外可达，见 §4.2） |
| **QQ 音乐** | `qqmusic`（**已实测确认**） | 下载页 `https://y.qq.com/download/download.html` 服务端渲染，HTML 内嵌版本 `1.1.8` 与重定向入口 `c.y.qq.com/cgi-bin/file_redirect.fcg?...&sign=...` → 最终主机 `dldir.y.qq.com` | ✅ HTML 模式可抓，下载实测通过（75,662,164 字节）；入口 URL 稳定、最终签名 URL 每次轮换但由入口重新生成 |
| **飞书** | `bytedance-feishu-stable`（**已实测确认**） | 接口 `https://www.feishu.cn/api/package_info?platform=<数字枚举>`（Linux x64 deb = `10`）返回**限时签名**下载链接 | ✅ 已攻破：`platform=10` 返回 `download_link` + `version_number`；签名 URL 有效 ~1h，.deb 354MB；**App 需在下载时重新调接口拿新鲜链接**（见 §5.4） |
| **腾讯会议** | `wemeet`（内置清单已收录，现为 `browserImport`） | 接口 `https://meeting.tencent.com/web-service/query-download-info?q=[...]` 返回 `url/version/size/md5` | ✅ 浏览器实测拿到 Linux .deb 地址（`updatecdn.meeting.qq.com`，206 可下）；接口的 `c_*` 参数需在生成器里复现 |
| **Obsidian** | `obsidian`（待 dpkg-deb 实测） | 下载页 `https://obsidian.md/download` 服务端渲染，HTML 内嵌 `obsidian_1.13.7_amd64.deb`（GitHub Releases 托管，但 `releases/latest` 只带手机 APK，不可靠） | ✅ HTML 模式可抓，版本从文件名解析；下载主机 `github.com`/`objects.githubusercontent.com` |
| **Bitwarden** | `bitwarden`（待 dpkg-deb 实测） | 稳定「最新 .deb」入口 `https://bitwarden.com/download/?app=desktop&platform=linux&variant=deb` → 302 GitHub `desktop-v2026.8.0/Bitwarden-2026.8.0-amd64.deb` → `release-assets.githubusercontent.com` | ✅ **固定最新 URL**，套 `stableDownloadEndpoint`（只需把 `page_version_marker` 做成可选，版本取 .deb 控制字段） |
| ~~网易云音乐~~ | — | 官方已无 Linux 端 | ❌ 不在范围 |

> **无需 `versionEndpoint` 的软件（已确认可走现有 kind）：**
> - **GitHub CLI**（`gh`）→ `aptRepository`，索引 `cli.github.com/packages`，已写入 `feed-sources.json`；
> - **LocalSend**（`localsend`）→ `releaseApi`，GitHub Releases 资产 `LocalSend-{tagVersion}-linux-x86-64.deb`，已实测（1.18.2+64 / amd64）并写入 `feed-sources.json`；
> - **Bitwarden**（`bitwarden`）→ `stableDownloadEndpoint`（固定最新 URL），见上表；改动仅「`page_version_marker` 可选化」。
>
> **超出 `.deb` 单包模型、暂不纳入：**
> - **LibreOffice**：Linux 官方分发是 `LibreOffice_*_Linux_x86-64_deb.tar.gz`（**多个 .deb 的 tar 包**，不是单 .deb），且 Ubuntu 官方源已内置。

> 说明：`packageName` 必须等于 `.deb` 控制文件里的 `Package` 字段（`dpkg-query` 扫描、helper 复核都依赖它）。QQ 已确认是 `linuxqq`；其余三家在接入前必须用 `dpkg-deb --field` 实测确认，不能猜。

## 3. 新增 source kind：`versionEndpoint`

### 3.1 数据模型（`crates/umanager-catalog/src/lib.rs`）

在 `SourceSpec` 枚举中新增一个变体（serde tag 为 `versionEndpoint`）：

```rust
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceSpec {
    // ...现有变体不变...

    #[serde(rename_all = "camelCase")]
    VersionEndpoint {
        /// 稳定 HTTPS 端点，返回 JSON / HTML。
        version_endpoint_url: String,
        /// 抓取端点时允许的精确域名（含重定向）。
        version_endpoint_hosts: Vec<String>,
        /// 端点载荷类型。
        payload_kind: VersionEndpointPayload,
        /// 端点请求附带的查询参数（如腾讯会议的 `q=[...]`、飞书的 `platform=...`）。
        #[serde(default)]
        query: Option<serde_json::Map<String, serde_json::Value>>,
        /// JSON 模式下：点分路径，取展示版本（支持数组下标，如 `info-list.0.version`）；HTML 模式下：版本文本标记。
        #[serde(default)]
        version_field: Option<String>,
        /// JSON 模式下：点分路径，取 .deb 下载地址（如 `Linux.x64DownloadUrl.deb` / `info-list.0.url`）；
        /// HTML 模式下：定位 .deb 链接的标记（如 `.deb` 后缀规则）。
        download_url_field: String,
        /// .deb 下载允许的精确域名（含重定向）。
        download_hosts: Vec<String>,
        /// 可选：对原始下载地址做签名/重定向处理（QQ 用，见 §4.2）。
        #[serde(default)]
        sign: Option<VersionEndpointSign>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VersionEndpointPayload {
    Json,
    /// 预留：JSONP / JS 包 JSON（QQ 旧页 `linuxConfig.js` 曾用）。
    JsonInScript,
    Html,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionEndpointSign {
    /// 签名接口（当前仅支持 `qqUrlSign`，即 trpc UrlSign/GetSign）。
    kind: VersionEndpointSignKind,
    endpoint_url: String,
    endpoint_hosts: Vec<String>,
    method: String,                    // "POST"
    /// 额外请求头（JSON 字符串或对象），例如 QQ 的 `x-oidb`。
    #[serde(default)]
    headers: Option<serde_json::Value>,
    /// 请求体模板，`{downloadUrl}` 占位符替换为原始下载地址。
    body_template: String,
    /// 响应 JSON 中取签名后地址的点分路径。
    signed_url_field: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VersionEndpointSignKind {
    QqUrlSign,
}
```

同时更新以下方法（新增分支即可，其余逻辑不变）：

- `is_auto_installable()`：`VersionEndpoint { .. }` 返回 `true`；
- `is_website_download()`：`VersionEndpoint { .. }` 返回 `true`（前端显示「官网直连」）；
- `download_hosts()`：`VersionEndpoint { download_hosts, .. }` 返回 `download_hosts`。

### 3.2 为什么 App 侧改动极小

关键事实：**主程序现在完全不抓厂商网站**。`source_engine.rs` 的下载/校验链路 100% 由 feed 驱动（`FeedApplicationEntry` 提供 `downloadUrl/size/sha256/version`）。source kind 只影响三件事：

1. 是否进入商店（`is_auto_installable()`）；
2. 前端展示「官方仓库 / 官网直连」（`is_website_download()`）；
3. 下载时允许哪些域名（`download_hosts()`）。

`feed_details` / `feed_plan` 里的 `matches!(app.source, SourceSpec::AptRepository { .. })` 会把新变体自然归入「官网直连」分支。`operation_plan.rs`、`installable.rs` 等也都通过上述方法间接工作。

> 例外（唯一需要动 App 的地方）：若 `source.sign` 存在（如 QQ），`source_engine.rs` 的下载路径需要在真正下载前**调用签名接口**把 feed 里的原始地址换成签名地址（见 §4.2）。这是 `download_plan_file` 里新增的一小段「签名 → 下载」步骤，不影响 helper / 计划 / 校验。

### 3.3 helper 为什么不需要改

- `website_applications()` 用 `application.is_auto_installable()` 过滤，新变体自动纳入 `InstallVerifiedWebsiteDeb` 授权路径；
- `feed_added_application()` 用同一个 `umanager_catalog::Application` 反序列化 `catalogJson`，新变体自动可解析；
- helper 的安装复核只看 `package/version/architecture/sha256/size` 与当前安装状态，不区分具体 source 细节。

> 安全不变量不被削弱：下载域名仍是**精确白名单**（`download_hosts`，禁止通配/前缀）；下载仍是 HTTPS + size + SHA-256 + .deb 元数据三重校验；计划仍不可变、15 分钟、归属当前用户；feed 新增软件仍走「内置公钥 + 计划内已签名 `catalogJson`」授权。

### 3.4 计划 schema 不受影响

plan schema 维持 **v2**。这类软件走现有的 `InstallVerifiedWebsiteDeb` 动作，与微信 / FlClash 完全一致。

## 4. CI 抓取器改动（`scripts/update-feed.mjs`）

在 `main()` 中新增分发分支：

```js
} else if (app.source?.kind === "versionEndpoint") {
  const entry = await versionEndpointEntry(app, app.source);
  if (entry) applications[app.applicationId] = entry;
}
```

新增 `versionEndpointEntry(app, source)`，流程：

1. `fetchText(source.versionEndpointUrl + 查询串)` 拉取端点（`query` 里每个键值 URL-encode 后拼接；`q` 这类值是 JSON 字符串，直接原样放入），校验最终域名 ∈ `version_endpoint_hosts`；
2. 按 `payload_kind` 解析：
   - `json`：`JSON.parse(text)`；
   - `jsonInScript`：定位第一个 `{`，按花括号配对（跳过字符串字面量）抽出 JSON 后 `JSON.parse`；
   - `html`：用 `version_field` 标记截取版本文本；用 `download_url_field` 规则截取 .deb 地址（`data-url="..."`、`href="..."`、裸 URL、以 `.deb` 结尾等）；
3. 用点分路径 `download_url_field` 从 JSON 取原始下载地址（支持 `info-list.0.url` 数组下标；HTML 模式直接得到地址）；
4. 若 `source.sign` 存在：请求签名接口，把 `{downloadUrl}` 替换进 `body_template`，从 `signed_url_field` 取签名后地址；
5. 校验下载地址：`https://` 且域名 ∈ `download_hosts`；
6. `downloadTemp()` 下载 .deb → `debControlField(Version)` 取控制版本 → `statSync().size` → `sha256OfFile()`；
7. 返回与 `stableDownloadEntry` / `releaseApiEntry` 同构的 entry（`websiteVersion` 用 `version_field` 解析出的展示版本）。

`downloadTemp` / `debControlField` / `sha256OfFile` / `compareVersions` 全部复用现有实现。`catalogJson = JSON.stringify(extraApplications)` 逻辑不变——新 source 描述符会随 `catalogJson` 一起被签名，helper 得以了解其 `download_hosts`。

### 4.1 版本优先级

与现有逻辑一致：**feed 的 `version` 取 .deb 控制文件里的 `Version`**（权威），`websiteVersion` 只是展示字段。因此即使官网展示版本格式与 Debian 版本不同，比较与计划生成仍用 Debian 版本。

### 4.2 QQ 的签名特例（trpc UrlSign，已用浏览器实测）

实测结论（`im.qq.com/index/#/linux` + DevTools 抓包）：

- 新官网拉取**纯 JSON**：`https://qq-web.cdn-go.cn/im.qq.com_new/latest/rainbow/pcConfig.json`；
- `Linux.x64DownloadUrl.deb` 里的**原始地址直连返回 403**（`qqdl.gtimg.cn/...deb`）；
- 官网会先调用签名接口（无鉴权、境外可达、返回 200）：
  - `POST https://im.qq.com/http2rpc/gotrpc/noauth/trpc.qqntv2.urlsign.UrlSign/GetSign`
  - 请求头 `Content-Type: application/json`、`x-oidb: {"uint32_command":"0x9b8e","uint32_service_type":1}`
  - 请求体 `{"url":"<原始 deb 地址>"}`
  - 响应 `{"data":{"url":"<原始地址>?sign=...&t=..."},"retcode":0}`
- **签名后的地址可下载（实测 206）**，即 `?sign=...&t=...` 是 QQ 的防盗链签名；
- 签名带 `t` 时间戳，**会过期**，因此 feed 里不能存固定签名地址。

**结论与分工：**

- **feed 只存原始地址**（`qqdl.gtimg.cn/...deb`）+ 由 CI 签名后下载算出的 size/sha256；
- **App 在下载时**（`source_engine` 的 `download_plan_file` 之前）用 `source.sign` 配置对原始地址调一次签名接口，拿到新鲜签名地址再下载；
- 下载最终域名仍必须 ∈ `download_hosts`（`qqdl.gtimg.cn`），并照旧走 https + size + SHA-256 + .deb 元数据校验。

对应 `sign` 配置（`signedUrlField` 为实测字段路径）：

```json
"sign": {
  "kind": "qqUrlSign",
  "endpointUrl": "https://im.qq.com/http2rpc/gotrpc/noauth/trpc.qqntv2.urlsign.UrlSign/GetSign",
  "endpointHosts": ["im.qq.com"],
  "method": "POST",
  "headers": { "Content-Type": "application/json", "x-oidb": "{\"uint32_command\":\"0x9b8e\",\"uint32_service_type\":1}" },
  "bodyTemplate": "{\"url\":\"{downloadUrl}\"}",
  "signedUrlField": "data.url"
}
```

> 注：签名接口主机 `im.qq.com` 也要进白名单（`endpointHosts`），但它只是换 token 的中间跳，.deb 本体仍只从 `download_hosts` 下载。

## 5. `feed-sources.json` 示例

### 5.1 QQ（数据源已实测确认）

```jsonc
{
  "applicationId": "qq",
  "packageName": "linuxqq",
  "displayName": "QQ",
  "vendor": "腾讯",
  "architecture": "amd64",
  "homepage": "https://im.qq.com/index/#/linux",
  "icon": "qq",
  "accentColor": "#12b7f5",
  "removable": true,
  "source": {
    "kind": "versionEndpoint",
    "versionEndpointUrl": "https://qq-web.cdn-go.cn/im.qq.com_new/latest/rainbow/pcConfig.json",
    "versionEndpointHosts": ["qq-web.cdn-go.cn"],
    "payloadKind": "json",
    "versionField": "Linux.version",
    "downloadUrlField": "Linux.x64DownloadUrl.deb",
    "downloadHosts": ["qqdl.gtimg.cn"],
    "sign": { /* 见 §4.2 */ }
  }
}
```

### 5.2 QQ 音乐（已实测：包名 / 版本 / 大小 / SHA-256 / 下载主机均确认）

```jsonc
{
  "applicationId": "qq-music",
  "packageName": "qqmusic",          // 已实测：dpkg-deb --field Package == qqmusic
  "displayName": "QQ 音乐",
  "vendor": "腾讯",
  "architecture": "amd64",
  "homepage": "https://y.qq.com/download/download.html",
  "icon": "qq-music",
  "accentColor": "#31c27c",
  "removable": true,
  "source": {
    "kind": "versionEndpoint",
    "versionEndpointUrl": "https://y.qq.com/download/download.html",
    "versionEndpointHosts": ["y.qq.com"],
    "payloadKind": "html",
    "versionField": "最新版:",             // 展示版本也可直接从文件名 qqmusic_1.1.8_amd64.deb 解析
    "downloadUrlField": ".deb",            // 规则：提取页面里首个以 .deb 结尾的 data-url/href
    "downloadHosts": ["c.y.qq.com", "dldir.y.qq.com"]
  }
}
```

实测数据（写入 feed 的字段来源）：

- `.deb` 控制字段：`Package: qqmusic`、`Version: 1.1.8`、`Architecture: amd64`；
- 大小：`75,662,164` 字节；
- SHA-256：`42d18d6a8c3c17414e5b6dbc0c3ac893bb2540c1c5c09ce90d241843cc593788`；
- 下载链路：入口 `c.y.qq.com/cgi-bin/file_redirect.fcg?...&sign=...` 302 → `dldir.y.qq.com/...?sign=<时间戳签名>`，最终签名每次不同但由入口实时生成，**入口 URL 稳定可复用**。

> 提取规则说明：页面里有 deb / AppImage / rpm 多个 `data-url`，用「值以 `.deb` 结尾」即可精确命中，无需脆弱的文本标记。`download_url_field` 在 HTML 模式下应理解为「URL 必须包含的子串 / 后缀规则」，而不是字面 marker。

> 风险：页面内嵌的入口 `sign`（`1-d1ca4d...-68cb7c1c` 形态）是否为长期有效仍需观察；若失效，CI 每次重抓页面即可刷新，影响仅限 feed 刷新频率。

### 5.3 腾讯会议（JSON API + 查询参数，已用浏览器实测）

`wemeet` 已在内置清单里（当前是 `browserImport`），升级为 `versionEndpoint` 即可自动安装。

```jsonc
{
  "applicationId": "wemeet",
  "packageName": "wemeet",          // 需用 dpkg-deb --field Package 实测确认
  "displayName": "腾讯会议",
  "vendor": "腾讯",
  "architecture": "amd64",
  "homepage": "https://meeting.tencent.com/download/",
  "icon": "wemeet",
  "accentColor": "#2878ff",
  "removable": true,
  "source": {
    "kind": "versionEndpoint",
    "versionEndpointUrl": "https://meeting.tencent.com/web-service/query-download-info",
    "versionEndpointHosts": ["meeting.tencent.com"],
    "payloadKind": "json",
    "query": {
      "q": "[{\"package-type\":\"app\",\"channel\":\"0300000000\",\"platform\":\"linux\",\"arch\":\"x86_64\",\"decorators\":[\"deb\"]}]"
    },
    "versionField": "info-list.0.version",
    "downloadUrlField": "info-list.0.url",
    "downloadHosts": ["updatecdn.meeting.qq.com"]
  }
}
```

浏览器实测（`query-download-info` 返回）：

- `version` = `3.26.10.401`；`size` = `197,268,516` 字节；
- `.deb` URL = `https://updatecdn.meeting.qq.com/cos/72e0e0023e1d1e6d4123fba28821aea1/TencentMeeting_0300000000_3.26.10.401_x86_64_default.publish.officialwebsite.deb`（206 可下）；
- 接口返回的是 **md5** 而非 SHA-256，生成器照旧下载 .deb 后用 `sha256OfFile` 计算。

> 待验证：接口在无浏览器 cookie 的 curl 下返回 `code:1`（浏览器带 cookie + 全套 `c_*` 参数时返回 `code:0`）。生成器需复现这些参数（`c_os`/`c_lang`/`c_timestamp`/`c_nonce`/`trace-id`/`rnds` 等），必要时带上 `web_uid` cookie；若仍被拒，考虑直接固定该 URL 并人工维护版本（退化方案）。

### 5.4 飞书（已攻破：数字枚举 + 限时签名 URL）

接口：`GET https://www.feishu.cn/api/package_info?platform=<数字枚举>`，注意**必须用数字枚举**（Linux x64 deb = `10`），用字符串 `LinuxX64Deb` 会返回 `{"code":1001,"message":"bind error"}`。

浏览器实测 `platform=10` 返回：

```json
{ "code": 0, "data": {
  "download_link": "https://lf9-ug-sign.feishucdn.com/ee-appcenter/fc38d53a/Feishu-linux_x64-7.72.23.deb?lk3s=...&x-expires=...&x-signature=...",
  "version_number": "Linux-x64-deb@V7.72.23",
  "qr_link": "",
  "weight": 7072023200
}}
```

- 版本：`7.72.23`（Debian 控制字段 `Version: 7.72.23-0`）；`.deb` 实际大小 **353,992,084** 字节（~354MB，`content-type: application/vnd.debian.binary-package`）；
- `.deb` 实测：`Package: bytedance-feishu-stable`、`Version: 7.72.23-0`、`Architecture: amd64`；
- SHA-256：`7c744ce101e29f50d8d0fc099788b64cb389dde3178178a7fce9ffcdbf296e13`；
- `download_link` 为**限时签名 URL**（`x-expires` 约 1 小时有效）；`weight` 字段不可信，实际大小以 Content-Length / sha256 校验为准；
- 数字枚举：`10`=Linux x64 deb、`11`=Linux x64 rpm、`12`=Linux ARM64 deb、`13`=Linux ARM64 rpm、`14`=Linux Mips64el deb、`15`=Linux Mips64el rpm、`16`=Windows x64。接口主机 `www.feishu.cn`，.deb 主机为 `lf{?}-ug-sign.feishucdn.com`。

> ⚠️ **下载主机前缀漂移风险（未解决）**：`.deb` 的 CDN 主机前缀**会变**——上一会话观察到 `lf9-ug-sign.feishucdn.com`，本次实测为 `lf6-ug-sign.feishucdn.com`。在「下载域名精确白名单、禁止通配/前缀匹配」的不变式下，这个漂移会导致 App 下载被拒（host 不在白名单）。当前实测稳定在 `lf6-ug-sign.feishucdn.com`，但长期来看前缀可能继续变。**这是飞书接入与不变式的直接冲突点**，接入前需定夺（见下一条）。

**架构影响**：签名 URL 有效 ~1h，而 feed 每 6h 刷新一次，所以 **feed 里的 downloadUrl 会过期**。App 必须在下载时**重新调 `package_info?platform=10`** 拿新鲜链接（类似 QQ 的签名子步骤，但这里是「整条 URL 都从接口拿」）。这需要一个「下载时重新解析下载地址」的机制——已实现为 `versionEndpoint.resolveAtDownload`（App 侧在 `source_engine::resolve_download_url` 里重新拉端点）。

### 5.5 Obsidian（HTML 模式，版本在文件名里）

```jsonc
{
  "applicationId": "obsidian",
  "packageName": "obsidian",          // 需用 dpkg-deb --field Package 实测确认
  "displayName": "Obsidian",
  "vendor": "Obsidian",
  "architecture": "amd64",
  "homepage": "https://obsidian.md/",
  "icon": "obsidian",
  "accentColor": "#7c3aed",
  "removable": true,
  "source": {
    "kind": "versionEndpoint",
    "versionEndpointUrl": "https://obsidian.md/download",
    "versionEndpointHosts": ["obsidian.md"],
    "payloadKind": "html",
    "versionField": "obsidian_",        // 版本从 .deb 文件名 obsidian_<version>_amd64.deb 解析
    "downloadUrlField": "_amd64.deb",   // 规则：提取页面里以 _amd64.deb 结尾的 URL
    "downloadHosts": ["github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com"]
  }
}
```

> 注意：Obsidian 的 `.deb` 在 GitHub Releases 上，但 `obsidianmd/obsidian-releases` 的 `releases/latest` 经常指向**只有手机 APK** 的发布（如 v1.13.8），桌面 .deb 却在 v1.13.7。所以不能用 `releaseApi`，用 HTML 模式抓 `obsidian.md/download` 更稳。

## 6. 改动清单汇总

| 文件 | 改动 | 是否触发发版 |
|---|---|---|
| `crates/umanager-catalog/src/lib.rs` | 新增 `VersionEndpoint` 变体 + `payload/sign` 子结构 + 三个方法分支 + 测试 | ✅（编译进主程序与 helper） |
| `scripts/update-feed.mjs` | 新增 `versionEndpointEntry` + 分发分支 + 解析器 | ❌（CI-only） |
| `src-tauri/src/source_engine.rs` | 新增「签名子步骤」：`source.sign` 存在时，在下载前调签名接口把 feed 原始地址换成签名地址（仅 QQ 需要） | ✅ |
| `crates/umanager-helper/src/main.rs` | 预期**零改动**（走 `is_auto_installable()` 与同 crate 反序列化） | — |
| `crates/umanager-plan` | 零改动（schema v2 不变） | — |
| `feed-sources.json` | 已加 GitHub CLI（aptRepository）、LocalSend（releaseApi）；后续追加 QQ / QQ 音乐 / Obsidian；腾讯会议改 `browserImport` → `versionEndpoint`；飞书待端点打通后追加 | ❌ |
| 图标 | `src/assets/app-icons/` 新增 `qq`、`qq-music`、`obsidian`、`github-cli`、`localsend`；`wemeet` 图标已存在 | ⚠️ 前端 `iconAssets` 是编译进包的，新图标**需要发版**才能显示（否则回退字母头像） |

> 注意：图标属于前端打包产物，加新图标和加 source kind 一样需要发版；正好和 v0.3.0 一起做。

## 7. 测试计划

1. **catalog crate**：`cargo test -p umanager-catalog`——新增：`versionEndpoint` 可反序列化、`is_auto_installable()` / `is_website_download()` / `download_hosts()` 行为正确、非法配置（空 hosts、非 https 端点）被拒；
2. **generator**：本地 `npm run update-feed`（需网络），确认 QQ 能产出 `applications.qq`，且 `.deb` 下载、控制版本、size、sha256 齐全；确认 `catalogJson` 含新条目；
3. **helper**：`cargo test -p umanager-helper`——用带 `catalogJson` 的计划走 `InstallVerifiedWebsiteDeb` 路径，确认 feed 新增的 `versionEndpoint` 应用被正常授权；
4. **全量**：`cargo check --manifest-path src-tauri/Cargo.toml` 零警告；`cargo test --manifest-path src-tauri/Cargo.toml`；`npm test`；`npm run build`；
5. **端到端**：发版后，商店应出现 QQ 等新软件，可下载校验、锁计划、dry-run、安装、卸载。

## 8. 发布步骤（v0.3.0）

1. 同步 bump 三处版本：`package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`；
2. `cargo check --manifest-path src-tauri/Cargo.toml`（同步 `Cargo.lock`）；
3. commit → `git tag -a v0.3.0` → push `main` → push tag；
4. `release.yml` 自动构建 `.deb`；随后 `gh workflow run update-feed` 让 feed 刷新。

## 9. 待办 / 开放问题

- [x] 实测 QQ：数据源 `pcConfig.json`（纯 JSON）、签名接口 `UrlSign/GetSign` 返回 `data.url`、签名地址可下载（206）、原始地址 403。
- [x] 实测 QQ 音乐：包名 `qqmusic`、版本 `1.1.8`、大小/SHA-256、下载主机 `dldir.y.qq.com`、入口 URL 稳定。
- [x] 实测腾讯会议：`query-download-info` 返回 Linux .deb（`updatecdn.meeting.qq.com`，206 可下）、版本 3.26.10.401、大小 197,268,516。
- [x] 实测 GitHub CLI：apt 索引 `cli.github.com/packages`、包 `gh` 2.98.0、池 URL 206 可下，已写入 `feed-sources.json`。
- [x] 实测 LocalSend：GitHub 资产 `LocalSend-1.18.2-linux-x86-64.deb`、控制包名 `localsend` 1.18.2+64、SHA-256 与 GitHub digest 一致，已写入 `feed-sources.json`。
- [x] 调研 Obsidian：`.deb` 在 GitHub Releases 但 `releases/latest` 不可靠（只带 APK），改用 HTML 模式抓 `obsidian.md/download`。
- [x] 调研 LibreOffice：Linux 为多 .deb 的 tar.gz，超出单 .deb 模型，暂不纳入。
- [x] 调研 Bitwarden：稳定入口 `bitwarden.com/download/?app=desktop&platform=linux&variant=deb` → GitHub `desktop-v2026.8.0/Bitwarden-2026.8.0-amd64.deb`（118,974,872 字节，`!<arch>` 确认为 .deb），固定最新 URL 模式。
- [ ] 确认 Bitwarden .deb 真实包名（预期 `bitwarden`，下载中）。
- [ ] 把 `StableDownloadEndpoint.page_version_marker` 改为可选（Bitwarden 无服务端渲染版本，版本取 .deb 控制字段）。
- [ ] 确认 Obsidian .deb 真实包名（预期 `obsidian`）。
- [ ] 确认腾讯会议 .deb 真实包名（预期 `wemeet`），并复现接口所需的 `c_*` 参数/ cookie（curl 下返回 `code:1`）。
- [x] 攻破飞书：`package_info` 用数字枚举 `platform=10`（字符串会 `bind error`），返回版本 7.72.23 + 限时签名 URL（有效 ~1h，.deb 354MB）。
- [x] 实测飞书 .deb：`Package: bytedance-feishu-stable`、`Version: 7.72.23-0`、sha256 `7c744ce1…`、大小 353,992,084。
- [ ] 决定飞书「下载时重新获取下载地址」的实现方式（`versionEndpoint` 的 `resolveAtDownload` 或类似）。
- [ ] 观察 QQ 签名地址 `?sign=...&t=...` 的过期窗口（确认 App 侧每次下载都需重新签名，还是可复用）。
- [ ] 观察 QQ 音乐页面内嵌入口 `sign` 的有效期（是否长期有效）。
- [ ] 确认新图标清单（`qq`、`qq-music`、`obsidian`、`github-cli`、`localsend`）与 accent 色值。
- [ ] 决定 HTML 模式下 `.deb` 链接标记的提取规则是否足够通用（还是按 app 加窄规则）。

---

## 附录：图标远端化（不把图标打进前端包）

> 背景：把图标硬编码进 `src/assets/app-icons/` 意味着每新增一个软件就要改 `App.tsx` 并发版，与「新增软件免更新 App」的目标矛盾。图标本质是应用元数据，应与版本信息一样由 feed 下发、App 拉取缓存。

### 目标行为

> 在 `feed-sources.json` 加一条 → CI 自动从该软件官方 `.deb` 里提取图标 → 与 feed 一起发布到 GitHub Pages → 老版本 App 拉取并缓存 → 直接显示图标。**全程零发版。**

### CI（`scripts/update-feed.mjs`）

1. 对每个受管应用，在已下载 .deb 校验的同一处，用 `dpkg-deb -x` 解包到临时目录；
2. 按优先级找图标（也验证了 LocalSend 可这样拿到）：
   - `/usr/share/icons/hicolor/{256x256,512x512,128x128,...}/apps/<app>.png`
   - `/usr/share/pixmaps/<app>.png`
   - 应用资源目录里的 `logo-*.png` / `icon.png`
3. 归一化为 PNG，写入 `dist/icons/<applicationId>.png`（`dist/` 由 `deploy-pages` 发布到 GitHub Pages，与 feed.json 同源）；
4. feed 的 `applications.<id>` 增加 `iconUrl`（`<feedBase>/icons/<id>.png`）与 `iconSha256`。

### App（主程序 + 前端）

1. 新增 Tauri 命令 `fetchAppIcon(appId, iconUrl, iconSha256)`：
   - 用 `restricted_client(metadataFeed.hosts)`（https-only + 域名白名单 = `null-object-0000.github.io`）拉取；
   - 校验 `iconSha256`；
   - 写缓存 `~/.cache/.../icons/<appId>-<sha256前16>.png`（按哈希命名，防止缓存投毒）。
2. 前端 `AppLogo` / `AppMark`：优先用「已缓存/远端图标」→ 否则回退内置 `iconAssets` → 再否则字母头像。
3. catalog 数据模型：`Application.icon` 改为可存放「内置 key 或完整 URL」（或新增 `icon_url` 字段），`icon_url` 走签名 feed。

### 安全不变量

- 图标与 feed **同源**（GitHub Pages，已进 `metadataFeed.hosts` 白名单）；
- **https-only**，`restricted_client` 域名精确匹配；
- **sha256** 校验（与 .deb 同强度）；
- 缓存按哈希命名、只读，大小上限（如 ≤ 512 KiB）；
- 内置 `iconAssets` 保留作旧版本/兜底回退，不强制迁移。

### 收益

- 新增软件图标零发版；
- 图标来源可审计（从厂商官方 .deb 提取，而非手拍）；
- 与「软件信息只来自签名 feed」的架构完全一致。

---

## 附录：软件/工具描述文案（已确认入 v0.3.0）

> 放法：`Application` / `DevelopmentTool` / `DevelopmentToolchain` 增加 `description: Option<String>`（可选，兼容旧数据）；内置的填 `vendors.json`，feed 新增的填 `feed-sources.json`（随签名 catalog 下发）。前端在商店行、详情抽屉、开发环境卡片展示。

### 软件商店（应用）

| 软件 | 描述 |
|---|---|
| **Visual Studio Code** | 微软出品的跨平台代码编辑器，内置 Git、智能补全与海量插件生态，是主流开发语言的首要选择。 |
| **Google Chrome** | 谷歌官方浏览器，渲染快、安全，支持丰富扩展与跨设备同步。 |
| **ChatGPT Desktop** | OpenAI 官方桌面客户端，把 ChatGPT 作为独立应用使用，无需依赖浏览器标签页。 |
| **FlClash** | 基于 Clash 内核的图形化代理客户端，管理订阅与节点，GitHub 官方发布。 |
| **微信** | 腾讯微信 Linux 桌面版，支持聊天、文件传输、公众号等日常功能。 |
| **腾讯会议** | 腾讯官方视频会议软件，支持云会议、屏幕共享、会议录制。 |
| **LocalSend** | 开源跨平台局域网文件传输工具，无需互联网即可在设备间安全互传文件，端到端加密。 |
| **QQ** | 腾讯 QQ Linux 桌面版，支持文字、语音、视频聊天与文件传输。 |
| **QQ音乐** | 腾讯官方音乐客户端，海量曲库与高品质音质，支持歌单与个性化推荐。 |
| **Obsidian** | 基于 Markdown 的本地知识库笔记应用，用双向链接构建个人知识网络，本地优先、数据自持。 |
| **Bitwarden** | 开源密码管理器，跨平台安全存储密码与敏感信息，端到端加密。 |
| **GitHub CLI** | GitHub 官方命令行工具，在终端里直接管理仓库、Issue、PR、Actions。 |

### 开发环境（工具链 + 开发工具）

| 工具 | 描述 |
|---|---|
| **Node.js** | JavaScript 运行时，前后端与工具链开发的基础，通过 nvm 管理多版本。 |
| **Rust** | 系统级编程语言，兼顾高性能与内存安全，通过 rustup 管理工具链。 |
| **Claude Code** | Anthropic 的终端 AI 编程助手，在命令行与 Claude 协作完成编码任务。 |
| **OpenCode** | 开源终端 AI 编程代理，在命令行中驱动 AI 完成编码任务。 |
| **Pi** | 开源终端编程代理，交互式命令行 AI 编码助手。 |
| **Codex CLI** | OpenAI 的命令行编程代理，在终端里执行与自动化代码任务。 |
