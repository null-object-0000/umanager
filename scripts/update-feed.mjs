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
import { mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { gunzipSync } from "node:zlib";

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(SCRIPT_DIR, "..");
const CATALOG_PATH = resolve(REPO_ROOT, "src-tauri/resources/vendors.json");
const OUT_PATH = process.argv[2] ?? resolve(REPO_ROOT, "dist", "feed.json");

const errors = [];

function log(message) {
  console.log(message);
}

function fail(context, message) {
  errors.push(`${context}: ${message}`);
  log(`!! ${context}: ${message}`);
}

async function fetchBuffer(url) {
  const response = await fetch(url, {
    redirect: "follow",
    headers: { "User-Agent": "UManager-feed/1.0" },
  });
  if (!response.ok) {
    throw new Error(`HTTP ${response.status}`);
  }
  return Buffer.from(await response.arrayBuffer());
}

async function fetchText(url) {
  return (await fetchBuffer(url)).toString("utf8");
}

async function fetchGz(url) {
  const buffer = await fetchBuffer(url);
  return (url.endsWith(".gz") ? gunzipSync(buffer) : buffer).toString("utf8");
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
  return {
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
}

async function releaseApiEntry(app, source) {
  let json;
  try {
    json = JSON.parse(await fetchText(source.releaseApiUrl));
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
  return {
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
}

async function stableDownloadEntry(app, source) {
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
  const displayVersion = afterMarker.slice(afterTag + 1).split("<")[0].trim();
  if (!displayVersion) {
    fail(app.applicationId, "官网展示版本解析为空");
    return null;
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
  return {
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

async function toolEntry(tool) {
  const pkg = tool.npmPackage;
  let json;
  try {
    json = JSON.parse(await fetchText(`https://registry.npmjs.org/${encodeURIComponent(pkg)}/latest`));
  } catch (error) {
    fail(tool.toolId, `读取 npm 最新版本失败：${error.message}`);
    return null;
  }
  if (!json.version) {
    fail(tool.toolId, `npm ${pkg} 未返回版本`);
    return null;
  }
  return { npmPackage: pkg, version: String(json.version) };
}

async function main() {
  const catalog = JSON.parse(readFileSync(CATALOG_PATH, "utf8"));

  const applications = {};
  for (const app of catalog.applications || []) {
    if (app.source?.kind === "aptRepository") {
      const entry = await aptEntry(app);
      if (entry) applications[app.applicationId] = entry;
    } else if (app.source?.kind === "releaseApi") {
      const entry = await releaseApiEntry(app, app.source);
      if (entry) applications[app.applicationId] = entry;
    } else if (app.source?.kind === "stableDownloadEndpoint") {
      const entry = await stableDownloadEntry(app, app.source);
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

  const feed = {
    schemaVersion: 1,
    generatedAtUnixSeconds: Math.floor(Date.now() / 1000),
    applications,
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
    `applications=${Object.keys(applications).length} selfUpdate=${selfUpdate !== null} tools=${Object.keys(developmentTools).length}`,
  );
  if (errors.length > 0) {
    log(`\n${errors.length} source(s) were skipped and fall back to on-device scraping:`);
    for (const e of errors) log(`  - ${e}`);
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
