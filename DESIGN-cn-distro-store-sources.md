# DESIGN：要不要用「统信 UOS / 银河麒麟应用商店」补充软件源

> 状态：**调研完成，建议「有条件接入、且只接一个源」，暂不实现。**
> 本文所有数字都是 2026-10-09 实测得到，第 10 节给了可重跑的复现命令。

## 0. 结论（TL;DR）

1. **四家里只有一家的应用目录是公开可取用的**：统信/deepin **社区**商店
   （`com-store-packages.uniontech.com/appstore`，40,710 个包）。统信 UOS **专业版**商店
   是 401 授权门槛，银河麒麟与 openKylin 的公开源是**纯操作系统基础源、一个商业应用都没有**。
2. **接进来在信任模型上不冲突**——它和现有 `aptRepository`（Microsoft / Google / Claude / WineHQ）
   是同一套语义：CI 读索引取版本与 SHA-256 → 我们自己签名进 feed → App 按签名 feed 校验。
   deepin 商店索引没有 GPG 签名，但现有四家 UManager 也没验 GPG，所以**不是新的削弱**。
3. **真正的卡点不是能不能下载，而是装不装得上**：
   - 商店 CDN 有**防盗链**，`UManager/0.1` 直接 403，要过就得伪造 apt UA 或商店 Referer；
   - 目录里 **82.4% 是 `uengine.*` 安卓重打包**、3.8% 是 deepin-wine 的 Windows 重打包，
     对 Ubuntu 用户无意义；
   - 剩下像样的包**几乎全部**被塞了一条 `deepin-elf-verify` 依赖，而 Ubuntu 没有这个包，
     它自己也补不上（见第 6 节）。**可用的池子是 11.5%。**
4. **WorkBuddy 这个具体案例不需要走商店**：UManager 已经在管它，且腾讯官方直供
   `copilot.tencent.com` 的通用 Linux deb，比从商店搬更干净（见第 3 节）。
5. 建议：**先只上「C 类」干净包做链路验证**；若要吃「A 类」那 4,667 个包，
   必须先做 plan v3 + helper 的「签名 feed 授权的可忽略依赖」设计（第 8 节选项 a）。

## 1. 动机

昨天发现 WorkBuddy 官方只支持统信 UOS / 银河麒麟，提示「请到系统应用商店下载」。
由此产生的问题：**能不能把国产系统的应用商店当成 UManager 的一个补充软件源？**

## 2. 实测：四家商店后端的公开可达性

| 商店 | 后端地址 | 公开可达 | WorkBuddy |
|---|---|---|---|
| 统信 UOS **专业版**商店 | `professional-packages.chinauos.com` | ❌ **HTTP 401**（需 UOS 许可授权） | 取不到 |
| 统信/deepin **社区**商店 | `com-store-packages.uniontech.com/appstore` | ⚠️ 索引公开，**deb 有防盗链** | ✅ `com.tencent.workbuddy` 5.6.2 |
| 银河麒麟 V10 SP1 | `archive.kylinos.cn/kylin/KYLIN-ALL` | ✅ 完全开放 | ❌ 全库 0 命中 |
| openKylin Nile 2.0 | `archive.openkylin.top/openkylin` | ✅ 完全开放 | ❌ 全库 0 命中 |

- 「请到系统应用商店下载」指的**专业版商店**恰恰是唯一 401 的那家。它需要 UOS 授权，
  不是技术抓取问题，是许可问题——**这家不要碰**。
- 麒麟与 openKylin 的公开源是**操作系统基础源**（Kylin main 10,217 包 / openKylin main 12,396 包）。
  按真实包名核对：微信、QQ、WPS Office、飞书、钉钉、WorkBuddy **全部 0 命中**
  （乍看的「命中」是 `libwps-0.4-4`、`qqwing`、`biometric-driver-wechat`、
  `qml-module-org-kde-qqc2*` 这类同名误报）。麒麟软件商店的域名
  （`store.kylinos.cn` / `appstore.kylinos.cn`）**公网不解析**。
  → 这两家目前**没有可被 UManager 取用的应用目录**。
- 所以实际候选只有**统信/deepin 社区商店**一家。

## 3. WorkBuddy 个案实测

从 deepin 社区商店拉到 `com.tencent.workbuddy` 并拆包：

| 项 | 值 |
|---|---|
| Version | `5.6.2` |
| Size | `391,371,194` 字节（373 MiB） |
| SHA-256 | `c6484a629da231c000834f44c1247725cc427fd15bae2b475e4b669e57ba9195` |
| 实测下载 SHA-256 | **与索引声明完全一致** ✅ |
| 是否有 maintainer scripts | **没有**（`control/` 里只有 `control` + `md5sums`） |
| Depends | `libgtk-3-0, libnotify4, libnss3, libxss1, libxtst6, xdg-utils, libatspi2.0-0, libuuid1, libsecret-1-0, deepin-elf-verify (>= 1.1.10-1)` |
| 载荷 | Electron 自包含：`/opt/apps/com.tencent.workbuddy/files/` 下自带 `node`(124MB)、`python3.13`(250MB)、`app.asar`(317MB)；主程序 ELF64 动态链接，解释器 `/lib64/ld-linux-x86-64.so.2` |
| desktop | `Exec=/opt/apps/com.tencent.workbuddy/files/workbuddy %U`，无 `X-Deepin-*`（不依赖 deepin 桌面集成） |

**两个关键推论：**

1. 除 `deepin-elf-verify` 外，其余依赖 Ubuntu 全都有；而包里**没有任何脚本调用它**。
   即 `deepin-elf-verify` 是**商店打包流水线注入的合成依赖**，不是真实运行时需求。
2. **但 UManager 现在也不需要它**：`feed-sources.json` 的 workbuddy 条目走
   `copilot.tencent.com/v2/update?platform=workbuddy-linux-x64-deb`，腾讯官方直供
   `WorkBuddy-linux-x64-deb-5.5.6.38337834-*.deb` 并自带 `sha256hash`。
   商店那份 5.6.2 是**带 UOS 定制适配的另一条构建线**（版本号体系不同，不是简单的新旧关系）；
   那些「系统登录联动 / 设备互联」的适配在 Ubuntu 上本来也不生效。

## 4. 防盗链实测矩阵

`com-store-packages.uniontech.com/appstore/pool/.../*.deb` 会 307 到
`app-store-files.uniontech.com/apppkg/<hash>.deb`，这一跳的行为：

| 客户端特征 | 结果 |
|---|---|
| `Referer: https://appstore.uniontech.com/`（带斜杠或不带） | ✅ 206 |
| `User-Agent: Debian APT-HTTP/1.3` / `Debian APT-HTTP/1.3 (2.6.1)` | ✅ 206 |
| `User-Agent: apt/2.6.1` | ❌ 403 |
| `User-Agent: Wget/1.21.3` | ❌ 403 |
| `User-Agent: curl/8.5.0` / `reqwest/0.12` / `deepin-app-store/6.0` | ❌ 403 |
| `User-Agent: UManager/0.1`（= 我们现在的客户端） | ❌ 403 |
| 浏览器 UA + `Referer: https://www.deepin.org/` | ❌ 403 |
| 浏览器 UA、无 Referer | ❌ 403 |

对照：`archive.kylinos.cn` 与 `archive.openkylin.top` 的 deb **裸下载 200**，无任何保护。

**这意味着**：接 deepin 商店必须**伪造 apt UA 或商店 Referer**，即主动绕过对方明确设置的反盗链。
技术上可行，但它①随时可能被对方收紧，②属于 ToS 灰区。这一点应当在决定时被显式承认，而不是当成普通技术适配。

另外索引 URL 本身也不稳定：`.../dists/deepin/appstore/binary-amd64/Packages.gz` 会 307 到
`app-store-files.uniontech.com/<token>/...`，`<token>`（实测 `261008181916319`）与该文件
Last-Modified（2026-10-08 18:19:16 +08:00）对应，**索引重新生成即变化**。
CI 侧 `update-feed.mjs` 用 fetch 默认跟随重定向尚可，但**不能把带 token 的路径写死进 feed**。

## 5. 目录构成：可用的只有 11.5%

对 40,710 个 stanza 按依赖分类（判定：`Pre-Depends`/`Depends` 里是否出现
`deepin* / dde-* / libdtk* / dtk* / uos* / ukui* / kylin* / uengine / linglong`）：

| 类别 | 数量 | 占比 | 说明 |
|---|---|---|---|
| A：**唯一障碍就是 `deepin-elf-verify`** | 4,667 | 11.5% | 扣掉这条假依赖即可装 → **唯一有意义的池子** |
| B：还有其他 deepin/UOS 绑定 | 35,590 | 87.4% | 其中 `uengine.*` 安卓重打包 **33,533**、deepin-wine 系 **1,536**、libdtk 系 **273** |
| C：完全没有 deepin/UOS 绑定 | 453 | 1.1% | 最干净，但量太小 |

A 类里比较像「UManager 值一提」的（体积最大的 12 个）：

```
pyvideotrans.app                         v3.69                2739MB
com.zwsoft.zw3d2027                      v2027.0.3.0          2644MB   ← 中望3D，无 Ubuntu 官方渠道
com.appimagehub.leillo1975.speed-dreams  v2.3.0               1899MB
top.gxde.gxde-lsg                        v1.0.2               1646MB
com.zwsoft.zw3d2026                      v2026.0.1.1          1643MB
com.jetbrains.rider                      v2024.3.6.2          1347MB
imageenhance                             v0.0.7               1333MB
com.jetbrains.www.idea                   v2025.2              1249MB
net.redeclipse.www                       v2.0.0               1241MB
en.st.stm32cubeide-installer             v1.16.0              1109MB
com.jetbrains.clion                       v2024.3.4.2         1090MB
tulinv8                                  v3.4.3               1085MB
```

**注意 A 类也不等于能跑**：这里只核了**声明的依赖**。glibc/libstdc++ 版本方向是安全的
（商店基于 Debian 12 一系，比 Ubuntu 24.04+ 旧，向前兼容），但「声明干净」不代表
「`/opt/apps/<id>` 的布局、图标路径、托盘集成在 Ubuntu 上都能正常工作」——那要逐个实测。

## 6. `deepin-elf-verify` 这条死路要单独说

它是这类包的**共同拦路虎**（6,784 个包的依赖里出现），而它自己补不上，闭环如下：

1. **deepin 社区商店索引里只有 1.1.5-2**（9,840 字节）→ **不满足** `>= 1.1.10-1`。
2. **deepin 主源里有 1.2.0.6-1**（`community-packages.deepin.com/deepin apricot main`，
   1,408 字节）→ 版本满足，**但它自己依赖 `libssl1.1`**。
3. **`libssl1.1` 在 Ubuntu jammy/noble 里都不存在**（Ubuntu 早已切到 libssl3）。
4. 也就是说，连「把假依赖真装上」这条兜底路也是死的。

所以要让 A 类包在 Ubuntu 上装成功，只剩三条路，都不轻：

- **(a) 计划里声明「可忽略的依赖」**：只针对**签名 feed 授权过的包名白名单**生效，
  由 helper 校验后对 `dpkg --install` 附加 `--force-depends` 语义。
  要动 `crates/umanager-plan`（plan v3）与 `crates/umanager-helper` 的固定 argv 不变量
  （AGENTS.md 约束 #7），是**安全敏感改动**，必须配反例单测。
- **(b) 自己重打包剥掉假依赖**：改变字节 → 破坏 feed 里的 SHA-256 锚点 → 得由 UManager 自己
  托管 deb → 变成**再分发**第三方专有软件，许可问题比 (a) 大得多。不建议。
- **(c) 只上 C 类**：453 个包、1.1%，收益太小，但可以零 schema 改动先跑通链路。

## 7. 若要接入：最小改动清单

复用现有 `aptRepository` kind（`DESIGN-multi-source.md` 的源注册表已预留位置）：

```jsonc
{
  "applicationId": "zw3d",
  "packageName": "com.zwsoft.zw3d2027",
  "sourceGroup": "deepin",              // 新增源组
  "source": {
    "kind": "aptRepository",
    "repositoryUrl": "https://com-store-packages.uniontech.com/appstore",
    "repositoryHosts": [
      "com-store-packages.uniontech.com",  // 索引与 pool 路径
      "app-store-files.uniontech.com"      // 307 之后的 CDN（必须加，否则重定向被客户端拒绝）
    ],
    "packagesIndexUrl": "https://com-store-packages.uniontech.com/appstore/dists/deepin/appstore/binary-amd64/Packages.gz"
  }
}
```

需要**新增**的能力（共两处，都不大）：

1. **自定义下载请求头**：`AptRepository` 增加可选 `downloadReferer` / `downloadUserAgent`。
   现状 `source_engine.rs` 的 `restricted_client()` 把 UA **硬编码**为 `UManager/0.1`（第 961 行），
   且没有注入自定义 header 的入口 → 不改必然 403。
2. **CI 侧同样的 header**：`scripts/update-feed.mjs::aptEntry()`（第 438 行）用
   `lastModifiedOf(downloadUrl)` 与图标提取下载 deb，同样会 403（非致命，但会丢版本时间与图标）。

需要**新增**的硬门槛（这条比上面两条重要）：

3. **上架前依赖核对门禁**：CI 对每个来自该源的包解析 `Depends`/`Pre-Depends`，
   命中 `deepin-* / dde-* / libdtk* / uengine / deepin-wine* / linglong` 就直接 fail 该条目。
   没有这条，App 端的 `dependency_check.rs` 会把这些包分到 `unavailable` 桶并提示
   「需自行添加厂商源」——而那对 Ubuntu 用户是**无解的死路**，正好落进该模块文件头
   自己写明的那个陷阱：*「一次装失败 → 卸载循环」*。

## 8. 三个选项与建议

| 选项 | 收益 | 代价 | 建议 |
|---|---|---|---|
| **(c) 只上 C 类（453 包）** | 跑通链路、零 schema 改动 | 量太小，用户感知弱 | ✅ **先做这个** |
| **(a) 上 A 类（4,667 包）** | 真正补充软件（中望3D 等确无 Ubuntu 渠道） | 要 plan v3 + helper 开 `--force-depends` 口子；要防盗链绕过 | ⏸ 先出专项设计再定 |
| (b) 重打包 | — | 再分发专有软件 + 破坏 SHA-256 锚点 | ❌ 不做 |

配套建议：

- 该源在 UI 上**默认关闭**，并明确标注「来自第三方国产系统商店，未在 Ubuntu 上逐一验证」，
  与 `DESIGN-multi-source.md` §6.3 的源卡片（信任状态 / 开关）天然契合。
- 防盗链绕过必须**显式记录为已知妥协**，不要伪装成普通适配；对方收紧时该源整体失效即可，
  不影响其他源（多源架构的隔离性正好兜住这点）。

## 9. 明确不做的

- **不碰统信 UOS 专业版商店**：401 是许可门槛不是技术门槛，绕过 = 法律风险 + 不稳定。
- **不整库镜像麒麟 / openKylin**：它们是操作系统基础源，里面没有商业应用，
  对 Ubuntu 用户零增量，只会让 feed CI 的时间和体积膨胀。
- **不把商店当成 WorkBuddy 的来源**：腾讯官方渠道已经在供，且更干净（第 3 节）。
- **不靠伪造 UA 长期跑**：可以应急，但要在设计里写明这是绕过对方反盗链控制。

## 10. 复现命令（证据可重跑）

```bash
# 1. 可达性
curl -sS -o /dev/null -w '%{http_code}\n' -L https://professional-packages.chinauos.com/desktop-professional/dists/eagle/main/binary-amd64/Packages.gz   # 401
curl -sSL -o /tmp/ds.gz https://com-store-packages.uniontech.com/appstore/dists/deepin/appstore/binary-amd64/Packages.gz                                # 200
curl -sS  -o /dev/null -w '%{http_code}\n' -L https://archive.kylinos.cn/kylin/KYLIN-ALL/dists/10.1/main/binary-amd64/Packages.gz                       # 200
curl -sS  -o /dev/null -w '%{http_code}\n' -L https://archive.openkylin.top/openkylin/dists/nile/main/binary-amd64/Packages.gz                          # 200

# 2. WorkBuddy 条目
gzip -cd /tmp/ds.gz | grep -A 20 '^Package: com.tencent.workbuddy$'

# 3. 防盗链矩阵（把 UA / Referer 换掉即可复现第 4 节）
U=https://com-store-packages.uniontech.com/appstore/pool/appstore/c/com.tencent.workbuddy/com.tencent.workbuddy_5.6.2_amd64.deb
curl -sS -o /dev/null -w '%{http_code}\n' -L -A 'UManager/0.1' -r 0-1023 "$U"                                     # 403
curl -sS -o /dev/null -w '%{http_code}\n' -L -A 'Debian APT-HTTP/1.3' -r 0-1023 "$U"                              # 206
curl -sS -o /dev/null -w '%{http_code}\n' -L -e 'https://appstore.uniontech.com/' -r 0-1023 "$U"                  # 206

# 4. 依赖闭环
gzip -cd /tmp/ds.gz | awk '/^Package: deepin-elf-verify$/{f=1} f&&/^(Version|Filename|Size):/{print} f&&/^$/{f=0}'   # 只有 1.1.5-2，不满足 >=1.1.10-1
curl -sSL https://community-packages.deepin.com/deepin/dists/apricot/main/binary-amd64/Packages.gz \
  | gzip -cd | awk '/^Package: deepin-elf-verify$/{f=1} f&&/^(Version|Depends):/{print} f&&/^$/{f=0}'               # 1.2.0.6-1，但依赖 libssl1.1（Ubuntu 无）
```

## 11. 短名单 v2：换成消费级口味（2026-10-09）

> **v1 已被否决**（OFD 阅读器 / 搜狗输入法 / Wind / 中望 CAD / 浩辰 CAD / Mind+ / CZUR 扫描仪 /
> 小蚁笔记 / 蓝信 / 亿图图示）。复盘：那一份整体偏**行业垂直 + 商业授权 + 硬件配套**，
> 共同点是「别人的工作要求」而不是「我想装的软件」。本版换成**消费级 / 日常 / 免费**的口味，
> 并补上了 v1 整个漏掉的两族：**AI 客户端**与**第三方移植客户端**。
> v1 的 A/C 分类与 `deepin-elf-verify` 结论仍然成立，只是选品换了。

### 11.1 本轮两个最重要的发现

1. **`*.uos` 那一族 AI 应用全是网页壳，不要接**。`com.doubao.uos` / `com.yiyan.uos` /
   `com.tongyi.uos` / `com.chatglm.uos` / `com.xinghuo.uos` / `com.360aisou.uos` 体积**一律 72.3MB**，
   实测拆包后 `resources/app.asar` **只有 2,855 字节**——72MB 全是原封不动的 Electron/Chromium 运行时，
   asar 里只是一个加载网页站的壳（desktop 文件还留着 `Categories=Development` 这种复制粘贴痕迹）。
   所以这**不是「豆包 Linux 版」，是「给豆包网页套了个窗口」**。接进来等于把浏览器书签做成应用，
   与 UManager「管理从厂商官网/官方仓库安装的 `.deb`」的定位相冲。**先用 11.1 的方法验 asar 大小，别被名字骗了。**
2. **第三方移植客户端是本商店里质量最高的一块**：`io.github.msojocs.*`（B 站、微信开发者工具）、
   `com.github.aliyunpan`（阿里云盘）、`vutron-music`（网易云）都是有真实功能的 Electron 应用，
   官方完全没有 Linux 端。**但它们不是厂商渠道**——接不接是产品定位决策，见 11.4。

### 11.2 v2 清单

| # | 包名 | 名称 | 版本 | 体积 | 类别 | 为什么是增量 |
|---|---|---|---|---|---|---|
| 1 | `cn.quark.quark-cloud-drive` | 夸克网盘 | 1.0.3-1 | 126MB | A（依赖为空） | 国民级网盘，官方**无** Linux 客户端；这是**原生移植**（描述明确写「由 win 版精简版移植，免去 wine 的烦恼」），不是网页壳 |
| 2 | `io.github.msojocs.wechat-devtools-linux` | 微信开发者工具 Linux | 2.02.2608070-2 | 186MB | A（依赖为空） | 小程序开发必备 IDE，官方只有 Windows/macOS；对 UManager 现有的开发者受众最贴合 |
| 3 | `io.github.msojocs.bilibili` | BiliBili For Linux | 1.19.0-1 | 133MB | A（依赖为空） | B 站桌面客户端，官方无 Linux 端；与已有的 LX Music（音乐）不重叠 |
| 4 | `com.todesk` | ToDesk | 4.7.2.0 | 60MB | A | 国产远程控制第一梯队，官方 Linux 版分发渠道有限 |
| 5 | `com.oray.sunlogin.client` | 向日葵 | 15.2.0.63062 | 79MB | A（仅 `libgconf-2-4`） | 另一大国产远控；与 ToDesk 互补，不同网络环境各有优势 |
| 6 | `com.snipaste.www` | Snipaste | 2.11.3 | 29MB | A（仅 `libfuse2`） | 口碑第一的截图工具；载荷是解开的 AppImage（`squashfs-root/`），**无任何 maintainer script** |
| 7 | `com.cbewin.anytxt` | Anytxt 文档内容搜索 | 1.3.3171 | 67MB | A（依赖为空） | 「文件内容版 Everything」，支持 Office/WPS/PDF/图片/代码全文索引，无 Linux 官方版 ⚠️ 见 11.5 |
| 8 | `enzh-dict` | 离线英汉词典 | 1.2.0 | 146MB | A（python3-tk 等，Ubuntu 都有） | UManager 没有词典类目；基于 ECDICT 340 万词条 + Tatoeba 例句，**完全离线** |
| 9 | `com.github.aliyunpan` | 阿里云盘客户端 | 3.11.26 | 85MB | A（依赖为空） | 阿里云盘官方**无** Linux 客户端，这是社区事实标准 ⚠️ 第三方 |
| 10 | `cn.deepseek.harness` | DeepSeek Harness | 0.1.0.7 | 67MB | A（仅 `systemd`） | 商店里有人把 DSH 打成了自带 Node.js 22 的独立 deb；但 UManager 已用 npm 方式管理 `dsh`，**这条更多是趣闻** |

### 11.3 落地顺序（换清单不改变工程前置条件）

- v2 的 #1 #2 #3 #4 #5 #6 #7 #8 #9 全部是 **A 类**（唯一障碍仍是 `deepin-elf-verify`），
  **依然卡在第 8 节选项 (a) 那个「签名 feed 授权的可忽略依赖」机制上——机制不做，这一版一个也上不了**。
- v2 里**没有 C 类**（v1 的 Mind+ 与 CZUR 是仅有的两个 C 类，都被否决了）。若坚持「零改动先跑通链路」，
  得从 11.6 的候选池里另挑 C 类。

### 11.4 一个比选品更重要的产品定位决策

UManager 的定位是「管理**从厂商官网/官方仓库**安装的 `.deb`」（AGENTS.md 约束 #1、README）。
v2 里的 #2 #3 #9（以及 #10）都是**第三方移植**，不是厂商渠道：

- 它们真实可用、官方确实没有 Linux 端，用户价值明确；
- 但会**打破「软件来自厂商官方渠道」的一致性**；且第三方客户端通常依赖非公开 API，
  随时可能因上游改接口失效，UManager 帮不上忙；
- 建议：**单开一个「社区移植」标记/分组**，在 UI 上明确区分来源；否则这一类就不接。

### 11.5 `com.cbewin.anytxt` 的包质量问题（不建议直接上）

实测拆包发现它的 `preinst`/`prerm` 写得很糙：

- 维护脚本里直接调 `sudo`（helper 已提权执行 dpkg，这是多余且危险的双重提权）；
- `pkill -9` 一串进程名；
- **有明显变量拼写错误**：`$ATGU!`、`$ATGUl` 之类，实际跑不到预期的程序；
- 往 deepin/麒麟专属目录写文件管理器右键插件
  （`/usr/share/deepin/dde-file-manager/oem-menuextensions`、`/usr/lib/x86_64-linux-gnu/peony-extensions`）
  ——在 Ubuntu 上这些分支靠「目录不存在」而跳过，属于**靠运气**而不是设计。

结论：**要么等厂商修，要么只进「实验源」且默认关闭**。

### 11.6 已验证项（实测下载 + 拆包）

| 包 | SHA-256 与索引一致 | 载荷 | 结论 |
|---|---|---|---|
| `com.doubao.uos` | ✅ `1a6f1efa…eeb5` | Electron，`app.asar` **仅 2,855B** | **网页壳，排除** |
| `com.cbewin.anytxt` | ✅ `bc4f925d…39d2` | 原生 `ATGUI` + 自有 lib | 脚本质量差，见 11.5 |
| `com.snipaste.www` | ✅ `c8aca1ff…1989` | 解开的 AppImage | 干净，仅 `libfuse2` |
| `xiaoyi-note`（v1） | ✅ `98bf503d…7ead` | Flutter，`/opt/xiaoyi-note/` | 干净 |
| `yozo-xreader`（v1） | ✅ `2ef6f0fd…aafa` | `/opt/Yozosoft/` | 干净 |
| `com.tencent.workbuddy` | ✅ `c6484a62…9195` | Electron，**无 maintainer script** | 干净 |

### 11.7 按品类的候选池（供继续挑选）

扫过 40,710 个包，**已排除** uengine 安卓重打包、deepin-wine、amber-ce 的 bubblewrap 容器壳、
`uos.app.*`/`uos.web.*` 网页壳、deepin 自家壁纸主题、以及本就有官方 Ubuntu 渠道的软件之后的剩余候选
（`A`=仅假依赖，`C`=完全干净）：

- **网盘/同步**：`cn.quark.quark-cloud-drive` 夸克 126MB(A)、`com.github.aliyunpan` 阿里云盘 85MB(A)、
  `github.nextcloud.nextcloud-desktop` 110MB(A)、`com.synology.drive` 群晖 Drive 81MB(A)、
  `site.datakeep` 267MB(A)、`com.gokuai.yunku` 够快云库 85MB(A)
- **远程控制**：`com.todesk` 60MB(A)、`com.oray.sunlogin.client` 向日葵 79MB(A)、
  `com.gitee.rustdesk` 19MB(A)、`cn.teamviewer.teamviewer` 70MB(**C**)、`com.chimeradesk` 22MB(A)、
  `net.devolutions` 168MB(A)
- **截图/录屏**：`com.snipaste.www` 29MB(A)、`sunny` 截图(A)、`com.obs-studio` OBS 77MB(A，官方有渠道)
- **搜索/效率**：`com.cbewin.anytxt` 67MB(A)、`com.github.chg-hou.dawnlightsearch` 21MB(A)、
  `dawnlightsearch`(A)、`github.hluk.copyq`(A，Ubuntu 有)
- **词典/阅读**：`enzh-dict` 146MB(A)、`cn.suwell.reader.appstore` 数科 OFD 106MB(A)、
  `reader` 道客阅读 120MB(A)、`com.jopdf` 209MB(A)、`com.calibre-ebook.calibre` 183MB(A，官方有)
- **音乐**：`vutron-music.stark81.github` 111MB(A)、`com.iease-music` 59MB(A)、
  `fun.upup.musicfree` 74MB(A)、`app.netlify.flb` 57MB(A)、`mplayer` 110MB(A)、
  `musicfans` 无损音乐/CD 抓轨 41MB(A)
- **视频/直播**：`io.github.msojocs.bilibili` 133MB(A)、`com.github.moonplayer` 290MB(A，依赖 bubblewrap)、
  `com.github.blogwy.bilibilivideodownload` 83MB(A)
- **AI 客户端**：`com.doubao.uos` / `com.yiyan.uos` / `com.chatglm.uos` / `com.xinghuo.uos` /
  `com.360aisou.uos` 等 72.3MB(A) —— **全是 2.8KB asar 网页壳，见 11.1**
- **开发**：`io.github.msojocs.wechat-devtools-linux` 186MB(A)、`org.electerm.electerm` 65MB(A)、
  `com.github.oxideterm` 56MB(A)、`cn.deepseek.harness` 67MB(A)、`com.navicat.premium` 140MB(A，商业)
- **安全**：`com.duba.antivirus` 金山毒霸 54MB(A)、`com.rising.antivirus.single` 瑞星 118MB(A)、
  `com.360.epp` 360 终端安全 170MB(A)、`com.charlesproxy.charles` 抓包 47MB(A)
- **办公/财务/其他**：`com.invoice.assistant` 智能发票助手 192MB(A)、`com.bingovue` 山海鲸可视化 193MB(A)、
  `com.wenku.baidu.uos` 百度文库 72MB(A)、`com.bsaippt.uos` 博思 AIPPT 73MB(A)、
  `com.gaoding.uos` 稿定 AI 73MB(A)、`com.360aigc.uos` 360 智绘 72MB(A)

## 12. 开放问题

- A 类包「声明干净 ≠ 能跑」：需要一个**逐包 Ubuntu 实机 smoke test** 的最小清单与流程，
  否则「默认关闭的实验源」仍可能给出错误信心。
- `deepin-elf-verify` 的冒名替换（选项 a 的 `--force-depends`）是否要限定为
  「仅忽略**声明中存在但被 feed 标注为 synthetic** 的依赖名」，而不是逐包白名单？
  逐包白名单更保守但维护成本高，倾向后者起步。
- 若未来麒麟/openKylin 开放应用目录（而非仅基础源），本文第 2 节的结论需要重测——
  它们的下载没有任何防盗链，接入成本比 deepin 商店低。
