// UManager metadata feed generator.
//
// This is the "software / version info scraping" that used to run inside the
// desktop app. It runs in GitHub Actions (see .github/workflows/update-feed.yml)
// and publishes a single feed.json to GitHub Pages. The UManager app now prefers
// this feed for candidate versions, sizes, SHA-256 digests and download URLs, and
// only falls back to on-device scraping when a source cannot be resolved here.
//
// For every source it is intentionally "best effort": a source that cannot be
// scraped right now is left out of the feed (the app falls back), never a fatal
// error for the whole run.
//
// Output schema:
//   schemaVersion: 1
//   generatedAtUnixSeconds: <seconds>
//   applications: { [applicationId]: { packageName, version, architecture,
//                                      size, sha256, downloadUrl, releaseTag?, assetName?, websiteVersion? } }
//   selfUpdate:   { packageName, version, architecture, size, sha256, downloadUrl, releaseTag?, assetName?, websiteVersion? }
//   developmentTools: { [toolId]: { npmPackage, version } }

import { spawnSync } from "node:child_process";
import { createHash, sign } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { mergeVersionUpdatedAt, parseLastModified, parseUnixSeconds } from "./version-time.mjs";
import { fileURLToPath } from "node:url";
import { gunzipSync } from "node:zlib";

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(SCRIPT_DIR, "..");
const CATALOG_PATH = resolve(REPO_ROOT, "src-tauri/resources/vendors.json");
const EXTRA_CATALOG_PATH = resolve(REPO_ROOT, "feed-sources.json");
const OUT_PATH = process.argv[2] ?? resolve(REPO_ROOT, "dist", "feed.json");

const errors = [];

function log(message) {
  console.log(message);
}

function fail(context, message) {
  errors.push(`${context}: ${message}`);
  log(`!! ${context}: ${message}`);
}

async function fetchBuffer(url, extraHeaders = {}) {
  const response = await fetch(url, {
    redirect: "follow",
    headers: { "User-Agent": "UManager-feed/1.0", ...extraHeaders },
  });
  if (!response.ok) {
    throw new Error(`HTTP ${response.status}`);
  }
  return Buffer.from(await response.arrayBuffer());
}

async function fetchText(url, extraHeaders) {
  return (await fetchBuffer(url, extraHeaders)).toString("utf8");
}

// GitHub API calls from a CI runner are unauthenticated by default (60/hr) and
// frequently 403 under runner shared IPs. When GITHUB_TOKEN is available (it is
// inside GitHub Actions) use it to raise the limit to 5000/hr. This token is
// never attached to non-GitHub vendor fetches.
function githubApiHeaders() {
  return process.env.GITHUB_TOKEN
    ? {
        Authorization: `Bearer ${process.env.GITHUB_TOKEN}`,
        Accept: "application/vnd.github+json",
      }
    : {};
}

async function fetchGz(url) {
  const buffer = await fetchBuffer(url);
  return (url.endsWith(".gz") ? gunzipSync(buffer) : buffer).toString("utf8");
}

// ---------------------------------------------------------------------------
// Gateway fallback: some vendor endpoints are only reachable from a China IP,
// so when a `gatewayUrl` is configured on a source and a direct fetch fails,
// retry through the Cloudflare Worker fetch proxy (`{gatewayUrl}/fetch?url=<url>`).
// Used only by the CI metadata generator — the desktop app never uses it.
// ---------------------------------------------------------------------------

function gatewayFetchUrl(gatewayUrl, url) {
  return `${gatewayUrl.replace(/\/+$/, "")}/fetch?url=${encodeURIComponent(url)}`;
}

async function fetchBufferFallback(url, gatewayUrl) {
  if (!gatewayUrl) return fetchBuffer(url);
  try {
    return await fetchBuffer(url);
  } catch (directError) {
    let host = url;
    try { host = new URL(url).hostname; } catch { /* keep raw */ }
    log(`  ↻ 直连失败（${directError.message}），改用网关抓取 ${host}`);
    return fetchBuffer(gatewayFetchUrl(gatewayUrl, url));
  }
}

async function fetchTextFallback(url, gatewayUrl) {
  return (await fetchBufferFallback(url, gatewayUrl)).toString("utf8");
}

async function downloadTempFallback(label, url, gatewayUrl) {
  const buffer = await fetchBufferFallback(url, gatewayUrl);
  const path = `/tmp/umanager-feed-${label}-${process.pid}.deb`;
  writeFileSync(path, buffer);
  return path;
}

// ---------------------------------------------------------------------------
// Version-update-time support: fetch the previous published feed as the state
// reference, and probe .deb Last-Modified for a server-modified timestamp.
// ---------------------------------------------------------------------------

async function fetchPreviousFeed(url) {
  if (!url) return null;
  try {
    return JSON.parse(await fetchText(url));
  } catch (error) {
    log(`  ↻ 读取上一版 feed 失败（${error.message}），按无历史处理`);
    return null;
  }
}

async function lastModifiedOf(url) {
  try {
    const response = await fetch(url, {
      method: "HEAD",
      redirect: "follow",
      headers: { "User-Agent": "UManager-feed/1.0" },
    });
    if (!response.ok) return null;
    return parseLastModified(response.headers.get("last-modified"));
  } catch {
    return null;
  }
}

function debControlField(filePath, field) {
  const result = spawnSync("dpkg-deb", ["--field", filePath, field], {
    encoding: "utf8",
  });
  if (result.status !== 0) {
    throw new Error(`dpkg-deb field ${field} failed: ${result.stderr?.trim()}`);
  }
  return result.stdout.trim();
}

function sha256OfFile(filePath) {
  const hash = createHash("sha256");
  hash.update(readFileSync(filePath));
  return hash.digest("hex");
}

// Numeric-aware Debian-ish version comparison so we can pick the highest candidate.
function compareVersions(a, b) {
  const split = (value) => value.split(/[.+-]/).filter(Boolean);
  const pa = split(a);
  const pb = split(b);
  for (let i = 0; i < Math.max(pa.length, pb.length); i += 1) {
    const xa = pa[i] ?? "0";
    const xb = pb[i] ?? "0";
    const na = Number(xa);
    const nb = Number(xb);
    if (Number.isFinite(na) && Number.isFinite(nb)) {
      if (na !== nb) return na - nb;
    } else if (xa !== xb) {
      return xa < xb ? -1 : 1;
    }
  }
  return 0;
}

function highest(records, key) {
  return records.reduce((best, current) => {
    if (!best) return current;
    return compareVersions(current[key], best[key]) > 0 ? current : best;
  }, null);
}

// Parse a Debian Packages file (paragraphs separated by blank lines).
function parsePackages(text, packageName, architecture) {
  const records = [];
  for (const paragraph of text.split(/\n\s*\n/)) {
    const fields = {};
    for (const line of paragraph.split("\n")) {
      const idx = line.indexOf(":");
      if (idx > 0) {
        const key = line.slice(0, idx).trim();
        const value = line.slice(idx + 1).trim();
        if (key && value) fields[key] = value;
      }
    }
    if (
      fields.Package === packageName &&
      fields.Architecture === architecture &&
      fields.Filename &&
      fields.Size &&
      fields.SHA256
    ) {
      records.push(fields);
    }
  }
  return records;
}

async function aptEntry(app) {
  const source = app.source;
  if (!source.packagesIndexUrl) {
    fail(app.applicationId, "apt 仓库未配置 packagesIndexUrl");
    return null;
  }
  let text;
  try {
    text = await fetchGz(source.packagesIndexUrl);
  } catch (error) {
    fail(app.applicationId, `读取 APT 索引失败：${error.message}`);
    return null;
  }
  const records = parsePackages(text, app.packageName, app.architecture);
  if (records.length === 0) {
    fail(app.applicationId, `APT 索引中没有 ${app.packageName}/${app.architecture}`);
    return null;
  }
  const chosen = highest(records, "Version");
  const downloadUrl = `${source.repositoryUrl.replace(/\/+$/, "")}/${chosen.Filename.replace(/^\/+/, "")}`;
  const serverModified = await lastModifiedOf(downloadUrl);
  const entry = {
    packageName: app.packageName,
    version: chosen.Version,
    architecture: app.architecture,
    size: Number(chosen.Size),
    sha256: chosen.SHA256.toLowerCase(),
    downloadUrl,
    releaseTag: null,
    assetName: null,
    websiteVersion: null,
  };
  entry._versionTimeCandidate = serverModified != null
    ? { time: serverModified, source: "serverModified" }
    : null;
  return entry;
}

async function releaseApiEntry(app, source) {
  let json;
  try {
    json = JSON.parse(await fetchText(source.releaseApiUrl, githubApiHeaders()));
  } catch (error) {
    fail(app.applicationId, `读取发布信息失败：${error.message}`);
    return null;
  }
  if (json.draft || json.prerelease) {
    fail(app.applicationId, "最新发布为草稿或预发布版本");
    return null;
  }
  const tagVersion = String(json.tag_name || "").replace(
    new RegExp(`^${escapeRegExp(source.stripTagPrefix || "")}`),
    "",
  );
  const expectedAssetName = (source.assetNamePattern || "").replace("{tagVersion}", tagVersion);
  const asset = (json.assets || []).find((item) => item.name === expectedAssetName);
  if (!asset || !asset.browser_download_url) {
    fail(app.applicationId, `发布中没有匹配资产 ${expectedAssetName}`);
    return null;
  }
  const digest = (asset.digest || "").replace(/^sha256:/, "");
  if (!/^[0-9a-f]{64}$/i.test(digest)) {
    fail(app.applicationId, `发布资产 ${asset.name} 缺少有效 SHA-256 摘要`);
    return null;
  }
  let windowFile;
  let controlVersion;
  try {
    windowFile = await downloadTemp(app.applicationId, asset.browser_download_url);
    controlVersion = debControlField(windowFile, "Version");
  } catch (error) {
    fail(app.applicationId, `读取发布资产控制信息失败：${error.message}`);
    return null;
  }
  const entry = {
    packageName: app.packageName,
    version: controlVersion,
    architecture: app.architecture,
    size: Number(asset.size),
    sha256: digest.toLowerCase(),
    downloadUrl: asset.browser_download_url,
    releaseTag: json.tag_name,
    assetName: asset.name,
    websiteVersion: tagVersion,
  };
  const officialTime = parseUnixSeconds(json.published_at ?? json.created_at);
  entry._versionTimeCandidate = officialTime != null
    ? { time: officialTime, source: "official" }
    : null;
  return entry;
}

async function stableDownloadEntry(app, source) {
  // The version marker is optional: when absent, skip scraping the page and use
  // the .deb control field's Version as the authoritative version (e.g. Bitwarden's
  // version-pinned "latest" URL, which has no server-rendered version text).
  let displayVersion = null;
  if (source.pageVersionMarker) {
    let html;
    try {
      html = await fetchText(source.officialPageUrl);
    } catch (error) {
      fail(app.applicationId, `读取官网页面失败：${error.message}`);
      return null;
    }
    const markerIndex = html.indexOf(source.pageVersionMarker);
    if (markerIndex < 0) {
      fail(app.applicationId, "官网页面中未找到版本节点");
      return null;
    }
    const afterMarker = html.slice(markerIndex + source.pageVersionMarker.length);
    const afterTag = afterMarker.indexOf(">");
    if (afterTag < 0) {
      fail(app.applicationId, "官网版本节点格式无效");
      return null;
    }
    displayVersion = afterMarker.slice(afterTag + 1).split("<")[0].trim();
    if (!displayVersion) {
      fail(app.applicationId, "官网展示版本解析为空");
      return null;
    }
  }

  // The vendor's download endpoint is fixed and configured in vendors.json; the
  // page is only used for the display version, not for the download URL.
  const downloadUrl = source.downloadUrl;
  if (!downloadUrl || !/^https:\/\//.test(downloadUrl)) {
    fail(app.applicationId, "官网下载地址无效");
    return null;
  }
  let windowFile;
  let controlVersion;
  try {
    windowFile = await downloadTemp(app.applicationId, downloadUrl);
    controlVersion = debControlField(windowFile, "Version");
  } catch (error) {
    fail(app.applicationId, `读取安装包控制信息失败：${error.message}`);
    return null;
  }
  const serverModified = await lastModifiedOf(downloadUrl);
  const entry = {
    packageName: app.packageName,
    version: controlVersion,
    architecture: app.architecture,
    size: statSync(windowFile).size,
    sha256: sha256OfFile(windowFile),
    downloadUrl,
    releaseTag: null,
    assetName: null,
    websiteVersion: displayVersion,
  };
  entry._versionTimeCandidate = serverModified != null
    ? { time: serverModified, source: "serverModified" }
    : null;
  return entry;
}

async function downloadTemp(label, url) {
  const buffer = await fetchBuffer(url);
  const path = `/tmp/umanager-feed-${label}-${process.pid}.deb`;
  writeFileSync(path, buffer);
  return path;
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

// ---------------------------------------------------------------------------
// Icon extraction: pull the app icon out of a vendor .deb without installing it.
// ---------------------------------------------------------------------------

function sha256Buf(buffer) {
  return createHash("sha256").update(buffer).digest("hex");
}

// Parse the PNG IHDR width/height (big-endian at offsets 16/20) to pick the
// largest icon resolution among candidates.
function pngDimensions(buffer) {
  if (buffer.length < 24 || buffer.readUInt32BE(12) !== 0x49484452) {
    return null; // not a PNG or missing IHDR
  }
  return { width: buffer.readUInt32BE(16), height: buffer.readUInt32BE(20) };
}

// Extract the best PNG icon from a .deb (dpkg-deb -x, no install). Returns
// { buffer, width, height } of the largest icon found, or null.
function extractIcon(debPath) {
  const extractDir = `/tmp/umanager-feed-icon-${process.pid}`;
  try {
    rmSync(extractDir, { recursive: true, force: true });
  } catch {
    /* ignore */
  }
  const result = spawnSync("dpkg-deb", ["-x", debPath, extractDir], {
    encoding: "utf8",
    stdio: "ignore",
  });
  if (result.status !== 0) {
    return null;
  }

  const candidates = [];
  for (const base of [
    join(extractDir, "usr/share/icons/hicolor"),
    join(extractDir, "usr/share/icons"),
    join(extractDir, "usr/share/pixmaps"),
  ]) {
    if (!existsSync(base)) continue;
    const walk = (dir) => {
      for (const entry of readdirSync(dir, { withFileTypes: true })) {
        const path = join(dir, entry.name);
        if (entry.isDirectory()) walk(path);
        else if (entry.name.toLowerCase().endsWith(".png")) candidates.push(path);
      }
    };
    walk(base);
  }
  if (candidates.length === 0) {
    return null;
  }

  let best = null;
  for (const path of candidates) {
    const buffer = readFileSync(path);
    const dimensions = pngDimensions(buffer);
    if (!dimensions) continue;
    const area = dimensions.width * dimensions.height;
    if (!best || area > best.area) {
      best = { buffer, area, width: dimensions.width, height: dimensions.height };
    }
  }
  return best;
}

// ---------------------------------------------------------------------------
// versionEndpoint: resolve a .deb download URL + version from a vendor endpoint
// that returns JSON / JSON-in-script / HTML.
// ---------------------------------------------------------------------------

function buildVersionEndpointUrl(source) {
  const url = new URL(source.versionEndpointUrl);
  for (const [key, value] of Object.entries(source.query || {})) {
    url.searchParams.set(key, typeof value === "string" ? value : JSON.stringify(value));
  }
  return url.toString();
}

// Dot-path access with array index support: "info-list.0.url" -> obj["info-list"][0]["url"].
function getJsonPath(obj, path) {
  if (!path) return undefined;
  let current = obj;
  for (const key of path.split(".")) {
    if (current == null) return undefined;
    if (Array.isArray(current) && /^\d+$/.test(key)) current = current[Number(key)];
    else current = current[key];
  }
  return current;
}

// Extract a balanced JSON object from inside a JS file (e.g. `var params = {...};`).
function extractJsonObject(text) {
  const start = text.indexOf("{");
  if (start < 0) throw new Error("未找到 JSON 对象");
  let depth = 0;
  let inStr = false;
  let esc = false;
  for (let i = start; i < text.length; i += 1) {
    const ch = text[i];
    if (inStr) {
      if (esc) esc = false;
      else if (ch === "\\") esc = true;
      else if (ch === '"') inStr = false;
    } else if (ch === '"') {
      inStr = true;
    } else if (ch === "{") {
      depth += 1;
    } else if (ch === "}") {
      depth -= 1;
      if (depth === 0) return text.slice(start, i + 1);
    }
  }
  throw new Error("JSON 对象不完整");
}

// Pick the .deb URL out of HTML: prefer a link ending in `.deb`, else the first
// URL whose path contains `.deb`.
function extractHtmlDebUrl(text) {
  const urls = text.match(/https:\/\/[^\s"'<>]+/g) || [];
  let fallback = null;
  for (const raw of urls) {
    const clean = raw.replace(/[),;]+$/, "");
    if (clean.endsWith(".deb")) return clean;
    if (!fallback && clean.includes(".deb")) fallback = clean;
  }
  return fallback;
}

// Extract the display version text that follows an HTML marker.
function extractHtmlVersion(text, marker) {
  if (!marker) return null;
  const index = text.indexOf(marker);
  if (index < 0) return null;
  return text.slice(index + marker.length).split(/[<"\s]/)[0] || null;
}

function hostAllowedInList(url, hosts) {
  try {
    const host = new URL(url).hostname;
    return hosts.some((allowed) => {
      if (allowed.startsWith("*.")) {
        const domain = allowed.slice(2);
        return host === domain || host.endsWith("." + domain);
      }
      return allowed === host;
    });
  } catch {
    return false;
  }
}

// Apply the configured URL-signing step (e.g. QQ's trpc UrlSign) to a raw .deb URL.
async function applySign(sign, rawUrl) {
  const headers = { "Content-Type": "application/json", ...(sign.headers || {}) };
  const body = sign.bodyTemplate.replace("{downloadUrl}", rawUrl);
  const response = await fetch(sign.endpointUrl, {
    method: sign.method || "POST",
    headers,
    body,
  });
  if (!response.ok) {
    throw new Error(`签名接口返回 HTTP ${response.status}`);
  }
  const json = JSON.parse(await response.text());
  const signed = getJsonPath(json, sign.signedUrlField);
  if (!signed) {
    throw new Error("签名接口未返回下载地址");
  }
  return signed;
}

async function versionEndpointEntry(app, source) {
  let text;
  try {
    text = await fetchTextFallback(buildVersionEndpointUrl(source), source.gatewayUrl);
  } catch (error) {
    fail(app.applicationId, `读取版本端点失败：${error.message}`);
    return null;
  }

  let rawUrl;
  let websiteVersion = null;
  try {
    if (source.payloadKind === "html") {
      rawUrl = extractHtmlDebUrl(text);
      websiteVersion = extractHtmlVersion(text, source.versionField);
    } else {
      const payload =
        source.payloadKind === "jsonInScript" ? JSON.parse(extractJsonObject(text)) : JSON.parse(text);
      rawUrl = getJsonPath(payload, source.downloadUrlField);
      websiteVersion = getJsonPath(payload, source.versionField) ?? null;
    }
  } catch (error) {
    fail(app.applicationId, `解析版本端点失败：${error.message}`);
    return null;
  }

  if (!rawUrl || !rawUrl.startsWith("https://")) {
    fail(app.applicationId, "版本端点未解析出有效的下载地址");
    return null;
  }
  if (!hostAllowedInList(rawUrl, source.downloadHosts || [])) {
    fail(app.applicationId, "下载地址不属于允许域名");
    return null;
  }

  let download = rawUrl;
  if (source.sign) {
    try {
      download = await applySign(source.sign, rawUrl);
    } catch (error) {
      fail(app.applicationId, `获取签名下载地址失败：${error.message}`);
      return null;
    }
  }

  let debPath;
  let controlVersion;
  try {
    debPath = await downloadTempFallback(app.applicationId, download, source.gatewayUrl);
    controlVersion = debControlField(debPath, "Version");
  } catch (error) {
    fail(app.applicationId, `读取安装包控制信息失败：${error.message}`);
    return null;
  }

  const entry = {
    packageName: app.packageName,
    version: controlVersion,
    architecture: app.architecture,
    size: statSync(debPath).size,
    sha256: sha256OfFile(debPath),
    downloadUrl: rawUrl,
    releaseTag: null,
    assetName: null,
    websiteVersion: websiteVersion ? String(websiteVersion) : null,
  };
  // JSON payload + configured releaseTimeField -> official publish time.
  entry._versionTimeCandidate = null;
  if (source.payloadKind !== "html" && source.releaseTimeField && payload) {
    const releaseTime = parseUnixSeconds(getJsonPath(payload, source.releaseTimeField));
    if (releaseTime != null) {
      entry._versionTimeCandidate = { time: releaseTime, source: "official" };
    }
  }
  return entry;
}

async function toolEntry(tool) {
  const pkg = tool.npmPackage;
  let doc;
  try {
    doc = JSON.parse(await fetchText(`https://registry.npmjs.org/${encodeURIComponent(pkg)}`));
  } catch (error) {
    fail(tool.toolId, `读取 npm 包信息失败：${error.message}`);
    return null;
  }
  const version = doc?.["dist-tags"]?.latest;
  if (!version) {
    fail(tool.toolId, `npm ${pkg} 未返回版本`);
    return null;
  }
  const entry = { npmPackage: pkg, version: String(version) };
  const publishTime = parseUnixSeconds(doc?.time?.[version]);
  entry._versionTimeCandidate = publishTime != null
    ? { time: publishTime, source: "official" }
    : null;
  return entry;
}

async function main() {
  const catalog = JSON.parse(readFileSync(CATALOG_PATH, "utf8"));

  let extra = { applications: [] };
  try {
    extra = JSON.parse(readFileSync(EXTRA_CATALOG_PATH, "utf8"));
  } catch (error) {
    if (error.code !== "ENOENT") {
      fail("feed-sources.json", `解析额外软件源失败：${error.message}`);
    }
  }
  const extraApplications = extra.applications || [];

  const applications = {};
  for (const app of [...(catalog.applications || []), ...extraApplications]) {
    if (app.source?.kind === "aptRepository") {
      const entry = await aptEntry(app);
      if (entry) applications[app.applicationId] = entry;
    } else if (app.source?.kind === "releaseApi") {
      const entry = await releaseApiEntry(app, app.source);
      if (entry) applications[app.applicationId] = entry;
    } else if (app.source?.kind === "stableDownloadEndpoint") {
      const entry = await stableDownloadEntry(app, app.source);
      if (entry) applications[app.applicationId] = entry;
    } else if (app.source?.kind === "versionEndpoint") {
      const entry = await versionEndpointEntry(app, app.source);
      if (entry) applications[app.applicationId] = entry;
    }
  }

  let selfUpdate = null;
  if (catalog.selfUpdate) {
    selfUpdate = await releaseApiEntry(
      {
        applicationId: "selfUpdate",
        packageName: catalog.selfUpdate.packageName,
        architecture: catalog.selfUpdate.architecture,
      },
      catalog.selfUpdate,
    );
  }

  const developmentTools = {};
  for (const tool of catalog.developmentTools || []) {
    const entry = await toolEntry(tool);
    if (entry) developmentTools[tool.toolId] = entry;
  }

  // Merge version-update-time for every app / self-update / dev tool. The
  // previous published feed is the state reference; entries never carry
  // `_versionTimeCandidate` into the signed output.
  const previousFeed = await fetchPreviousFeed(catalog.metadataFeed?.url);
  const nowUnixSeconds = Math.floor(Date.now() / 1000);
  const applyVersionTime = (entry, previousEntry) => {
    if (!entry) return;
    const candidate = entry._versionTimeCandidate || null;
    const merged = mergeVersionUpdatedAt(
      previousEntry ?? null,
      entry.version,
      candidate,
      nowUnixSeconds,
    );
    entry.versionUpdatedAtUnixSeconds = merged?.time ?? null;
    entry.versionUpdatedAtSource = merged?.source ?? null;
    delete entry._versionTimeCandidate;
  };
  for (const [id, entry] of Object.entries(applications)) {
    applyVersionTime(entry, previousFeed?.applications?.[id]);
  }
  if (selfUpdate) applyVersionTime(selfUpdate, previousFeed?.selfUpdate);
  for (const [id, entry] of Object.entries(developmentTools)) {
    applyVersionTime(entry, previousFeed?.developmentTools?.[id]);
  }

  // For versionEndpoint sources, the feed `downloadUrl` is not always directly
  // downloadable (QQ: raw URL needs URL-signing; Feishu: signed link expires).
  // Resolve a fresh downloadable URL so icon extraction reuses the same logic
  // as entry generation (sign / resolveAtDownload), otherwise these icons 4xx.
  async function iconDownloadUrl(app, entry) {
    const source = app.source;
    if (source?.kind !== "versionEndpoint") return entry.downloadUrl;
    let url = entry.downloadUrl;
    if (source.resolveAtDownload) {
      const text = await fetchTextFallback(buildVersionEndpointUrl(source), source.gatewayUrl);
      const payload = source.payloadKind === "html"
        ? null
        : JSON.parse(source.payloadKind === "jsonInScript" ? extractJsonObject(text) : text);
      if (payload) url = getJsonPath(payload, source.downloadUrlField);
    }
    if (source.sign) url = await applySign(source.sign, url);
    return url;
  }

  // Extract + publish icons for feed-added applications, and inject the icon
  // URL + SHA-256 into the signed catalog. Built-in apps keep their bundled icons.
  const iconBase = catalog.metadataFeed?.url?.replace(/\/[^/]*$/, "");
  const iconsDir = join(dirname(OUT_PATH), "icons");
  if (iconBase && extraApplications.length > 0) {
    mkdirSync(iconsDir, { recursive: true });
    for (const app of extraApplications) {
      const entry = applications[app.applicationId];
      if (!entry?.downloadUrl) {
        log(`  icon: ${app.applicationId} — 无下载地址，跳过`);
        continue;
      }
      try {
        const debPath = await downloadTemp(app.applicationId, await iconDownloadUrl(app, entry));
        const icon = extractIcon(debPath);
        rmSync(debPath, { force: true });
        if (!icon) {
          log(`  icon: ${app.applicationId} — 未找到图标`);
          continue;
        }
        const iconPath = join(iconsDir, `${app.applicationId}.png`);
        writeFileSync(iconPath, icon.buffer);
        // catalogJson uses camelCase to match the serde Application model.
        app.iconUrl = `${iconBase}/icons/${app.applicationId}.png`;
        app.iconSha256 = sha256Buf(icon.buffer);
        log(`  icon: ${app.applicationId} ✓ (${icon.width}x${icon.height})`);
      } catch (error) {
        fail(app.applicationId, `图标提取失败：${error.message}`);
      }
    }
  }

  // `gatewayUrl` is a CI-only config; strip it from the signed catalog so the
  // desktop app / helper never see the private gateway endpoint.
  for (const app of extraApplications) {
    if (app.source && "gatewayUrl" in app.source) delete app.source.gatewayUrl;
  }

  const catalogJson = JSON.stringify(extraApplications);
  const catalogSignature = process.env.FEED_SIGNING_KEY
    ? sign(null, Buffer.from(catalogJson), process.env.FEED_SIGNING_KEY).toString("hex")
    : null;

  const feed = {
    schemaVersion: 2,
    generatedAtUnixSeconds: Math.floor(Date.now() / 1000),
    applications,
    catalogJson,
    catalogSignature,
    selfUpdate,
    developmentTools,
  };

  mkdirSync(dirname(OUT_PATH), { recursive: true });
  const feedBytes = Buffer.from(JSON.stringify(feed, null, 2));
  writeFileSync(OUT_PATH, feedBytes);
  log(`Wrote ${OUT_PATH}`);

  if (process.env.FEED_SIGNING_KEY) {
    const signature = sign(null, feedBytes, process.env.FEED_SIGNING_KEY);
    writeFileSync(`${OUT_PATH}.sig`, signature.toString("hex"));
    log(`Wrote ${OUT_PATH}.sig (Ed25519)`);
  } else {
    log("FEED_SIGNING_KEY not set — feed published unsigned");
  }

  log(
    `applications=${Object.keys(applications).length} extraCatalog=${extraApplications.length} selfUpdate=${selfUpdate !== null} tools=${Object.keys(developmentTools).length}`,
  );
  if (errors.length > 0) {
    log(`\n${errors.length} source(s) were skipped and will be unavailable in the feed:`);
    for (const e of errors) log(`  - ${e}`);
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
