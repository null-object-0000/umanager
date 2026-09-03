// UManager metadata feed generator.
//
// This is the "software / version info scraping" that used to run inside the
// desktop app. It runs in GitHub Actions (see .github/workflows/update-feed.yml)
// and publishes a single feed.json to GitHub Pages. The UManager app now prefers
// this feed for candidate versions, sizes, SHA-256 digests and download URLs, and
// only falls back to on-device scraping when a source cannot be resolved here.
//
// For every source it is intentionally "best effort": a source that cannot be
// scraped right now reuses its previous published entry when one exists, and is
// only left out of the feed when it was never in the feed before. A single
// failed scrape is never a fatal error for the whole run.
//
// Modes (see DESIGN-multi-source.md §7):
//   default            single-pass full generation (local `npm run update-feed`)
//   --group <id> --out <path>
//                      scrape one source group and publish its final, signed
//                      source feed (feed.<id>.json: that group's apps + its
//                      own signed catalog). Run in parallel, one job per group.
//   --merge --parts <dir> --out <path>
//                      aggregate every source feed in <dir>/<group>/feed.<group>.json
//                      into the central feed.json (canonical app order, full
//                      catalog, selfUpdate + development tools, previous-feed
//                      fallback) and copy the source feeds into the output dir
//                      so Pages hosts them alongside the central feed.
//
// Output schema:
//   schemaVersion: 2
//   generatedAtUnixSeconds: <seconds>
//   applications: { [applicationId]: { packageName, version, architecture,
//                                      size, sha256, downloadUrl, releaseTag?, assetName?, websiteVersion?,
//                                      releaseNotes?, releaseNotesUrl? } }
//   selfUpdate:   { packageName, version, architecture, size, sha256, downloadUrl, releaseTag?, assetName?, websiteVersion?,
//                   releaseNotes?, releaseNotesUrl? }
//   developmentTools: { [toolId]: { npmPackage?, version } }  // npmPackage omitted for non-npm tools
//   categories: [ { id, label } ]          // display-only grouping
//   categoryAssignments: { applications: { [applicationId]: categoryId },
//                          developmentTools: { [toolId]: categoryId } }

import { spawnSync } from "node:child_process";
import { createHash, randomBytes, sign } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { atomChangelog } from "./changelog-atom.mjs";
import { extractHtmlBlockToMarkdown, extractHtmlVersionSection, htmlChangelogToMarkdown, parseHtmlChangelog, parseHtmlVersionList } from "./changelog-html.mjs";
import { jsonpEntryToMarkdown, parseJsonpChangelog, selectJsonpChangelogEntry } from "./changelog-jsonp.mjs";
import { cleanReleaseNotesMarkdown, extractMarkdownVersionSection } from "./changelog-markdown.mjs";
import { entryOrPrevious } from "./feed-fallback.mjs";
import { applyVersionTime, mergeSourceFeeds, parseCatalogApplications, sourceGroupOf as sourceGroupOfApp, validateSourceGroups } from "./feed-merge.mjs";
import { sanitizeReleaseNotes, selectReleaseNotesRelease, selectToolRelease, stripReleaseNotesBoilerplate } from "./release-notes.mjs";
import { resolveNpmDistTagVersion } from "./tool-version.mjs";
import { mergeVersionUpdatedAt, parseLastModified, parseUnixSeconds } from "./version-time.mjs";
import { fileURLToPath } from "node:url";
import { gunzipSync } from "node:zlib";

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(SCRIPT_DIR, "..");
const CATALOG_PATH = resolve(REPO_ROOT, "src-tauri/resources/vendors.json");
const EXTRA_CATALOG_PATH = resolve(REPO_ROOT, "feed-sources.json");
const CATEGORY_PATH = resolve(REPO_ROOT, "feed-categories.json");

// Source groups (CI-only): which apps are scraped by which generate job, and
// later published as a distinct feed source (DESIGN-multi-source.md §7).
// Built-in (vendors.json) apps are mapped here so vendors.json stays an
// app-compiled artifact; feed-sources.json apps carry their own `sourceGroup`
// field. `selfUpdate` and development tools always stay in the central feed
// (their endpoints — GitHub / npm — are fast, so splitting them adds nothing).
const SOURCE_GROUPS = { tencent: ["wechat", "wemeet"] };
const DEFAULT_SOURCE_GROUP = "common";
// Bounded concurrency for vendor fetches: network-bound, so a few parallel
// workers collapse the sequential scrape without hammering vendor endpoints.
const CONCURRENCY = 5;

function sourceGroupOf(app) {
  return sourceGroupOfApp(app, SOURCE_GROUPS, DEFAULT_SOURCE_GROUP);
}

// Downloaded .deb cache keyed by applicationId: entry generation downloads a
// vendor .deb once (control version / size / SHA-256) and icon extraction
// reuses the same file instead of downloading it a second time.
const downloadedDebs = new Map();

const errors = [];

function log(message) {
  console.log(message);
}

function fail(context, message) {
  errors.push(`${context}: ${message}`);
  log(`!! ${context}: ${message}`);
}

function parseArgs(argv) {
  const args = { group: null, merge: false, out: null, parts: null, positional: null };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--group") args.group = argv[++index] ?? null;
    else if (arg === "--merge") args.merge = true;
    else if (arg === "--out") args.out = argv[++index] ?? null;
    else if (arg === "--parts") args.parts = argv[++index] ?? null;
    else if (arg.startsWith("-")) throw new Error(`未知参数：${arg}`);
    else args.positional = arg;
  }
  return args;
}

// Run `fn` over `items` with at most `limit` concurrent workers, preserving
// input order. `fn` is expected to catch its own errors (the entry scrapers
// fail() and return null); a thrown error becomes a failed item.
async function mapLimit(items, limit, fn) {
  const results = new Array(items.length);
  let nextIndex = 0;
  const workers = Array.from({ length: Math.min(limit, items.length) }, async () => {
    while (nextIndex < items.length) {
      const index = nextIndex;
      nextIndex += 1;
      try {
        results[index] = await fn(items[index], index);
      } catch (error) {
        results[index] = null;
      }
    }
  });
  await Promise.all(workers);
  return results;
}

// Best-effort read of the display-only category config. Missing/invalid config
// degrades to an empty block: items simply fall back to the app's built-in
// mapping or the "其他" bucket — never a fatal error for the feed run.
function readCategoryConfig() {
  try {
    return JSON.parse(readFileSync(CATEGORY_PATH, "utf8"));
  } catch (error) {
    if (error.code !== "ENOENT") fail("feed-categories.json", `解析分类配置失败：${error.message}`);
    return {};
  }
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
  downloadedDebs.set(label, path);
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
  // `~` is Debian's pre-release / suffix separator — WineHQ ships versions
  // like `11.16~resolute-1` to tag the distro build target. Without splitting
  // on it, "11.16~resolute-1" vs "11.9~resolute-1" falls back to opaque string
  // comparison ("16~resolute" < "9~resolute") and picks the wrong candidate.
  // With the split, "11.16" sorts above "11.9" numerically as intended.
  const split = (value) => value.split(/[.+\-~]/).filter(Boolean);
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
  const releaseNotesUrl = typeof json.html_url === "string" && /^https:\/\//.test(json.html_url)
    ? json.html_url
    : null;
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
    releaseNotes: sanitizeReleaseNotes(stripReleaseNotesBoilerplate(json.body, app.applicationId)),
    releaseNotesUrl,
  };
  const officialTime = parseUnixSeconds(json.published_at ?? json.created_at);
  entry._versionTimeCandidate = officialTime != null
    ? { time: officialTime, source: "official" }
    : null;
  return entry;
}

// Fetch release notes from a GitHub Releases endpoint, an Atom changelog feed,
// or a versioned HTML / Markdown updates page configured on the app, independent
// of the download source kind. This is what lets non-`releaseApi` sources
// (aptRepository / stableDownloadEndpoint / versionEndpoint) ship a changelog
// through the signed feed. Best-effort: a failure only leaves the note absent,
// it never drops the app entry.
async function fetchReleaseNotes(app, config, entry) {
  if (!config || typeof config !== "object") return null;
  if (typeof config.atomUrl === "string") return fetchAtomReleaseNotes(app, config);
  if (typeof config.versionedHtmlUrl === "string") return fetchVersionedHtmlReleaseNotes(app, config, entry);
  if (typeof config.versionedMarkdownUrl === "string") return fetchVersionedMarkdownReleaseNotes(app, config, entry);
  if (typeof config.changelogHtmlUrl === "string") return fetchChangelogHtmlReleaseNotes(app, config, entry);
  if (typeof config.changelogListHtmlUrl === "string") return fetchChangelogListHtmlReleaseNotes(app, config, entry);
  if (typeof config.changelogBlockHtmlUrl === "string") return fetchChangelogBlockHtmlReleaseNotes(app, config);
  if (typeof config.changelogJsonpUrl === "string") return fetchChangelogJsonpReleaseNotes(app, config, entry);
  if (typeof config.releaseApiUrl !== "string") return null;
  return fetchGitHubReleaseNotes(app, config);
}

// Resolve `{version}`, `{major}`, `{minor}`, `{patch}` placeholders in a URL
// template from the resolved version (e.g. `1.135.0-1787669172` → major `1`,
// minor `135`). Values are percent-encoded.
function resolveVersionPlaceholders(template, version) {
  if (typeof template !== "string") return template;
  const parts = String(version).split(".");
  const major = parts[0] ?? "";
  const minor = parts[1] ?? "";
  const patch = parts[2] ?? "";
  return template
    .replaceAll("{version}", encodeURIComponent(String(version)))
    .replaceAll("{major}", encodeURIComponent(major))
    .replaceAll("{minor}", encodeURIComponent(minor))
    .replaceAll("{patch}", encodeURIComponent(patch));
}

async function fetchVersionedHtmlReleaseNotes(app, config, entry) {
  const version = config.versionField === "version"
    ? entry?.version
    : (entry?.websiteVersion ?? entry?.version);
  if (!version) return null;
  const url = resolveVersionPlaceholders(config.versionedHtmlUrl, version);
  let html;
  try {
    html = await fetchText(url);
  } catch (error) {
    log(`  releaseNotes: ${app.applicationId} — ${error.message}`);
    return null;
  }
  const releaseNotes = sanitizeReleaseNotes(
    stripReleaseNotesBoilerplate(htmlChangelogToMarkdown(parseHtmlChangelog(html)), app.applicationId),
  );
  const releaseNotesUrl = /^https:\/\//.test(url) ? url : null;
  if (!releaseNotes && !releaseNotesUrl) return null;
  return { releaseNotes, releaseNotesUrl };
}

async function fetchVersionedMarkdownReleaseNotes(app, config, entry) {
  const version = config.versionField === "version"
    ? entry?.version
    : (entry?.websiteVersion ?? entry?.version);
  if (!version) return null;
  const url = resolveVersionPlaceholders(config.versionedMarkdownUrl, version);
  let text;
  try {
    text = await fetchText(url);
  } catch (error) {
    log(`  releaseNotes: ${app.applicationId} — ${error.message}`);
    return null;
  }
  const releaseNotes = sanitizeReleaseNotes(
    stripReleaseNotesBoilerplate(cleanReleaseNotesMarkdown(text), app.applicationId),
  );
  const notesUrl = config.releaseNotesUrl
    ? resolveVersionPlaceholders(config.releaseNotesUrl, version)
    : url;
  const releaseNotesUrl = /^https:\/\//.test(notesUrl) ? notesUrl : null;
  if (!releaseNotes && !releaseNotesUrl) return null;
  return { releaseNotes, releaseNotesUrl };
}

// Fetch release notes from a fixed VitePress-style changelog page (CodeBuddy):
// the page holds every version as `<h3>X.Y.Z (date)</h3>` sections. Select the
// section matching the resolved version, else the first (latest) section.
async function fetchChangelogHtmlReleaseNotes(app, config, entry) {
  const version = config.versionField === "version"
    ? entry?.version
    : (entry?.websiteVersion ?? entry?.version);
  if (!version) return null;
  let html;
  try {
    html = await fetchText(config.changelogHtmlUrl);
  } catch (error) {
    log(`  releaseNotes: ${app.applicationId} — ${error.message}`);
    return null;
  }
  const releaseNotes = sanitizeReleaseNotes(
    stripReleaseNotesBoilerplate(extractHtmlVersionSection(html, version), app.applicationId),
  );
  const notesUrl = config.releaseNotesUrl ?? config.changelogHtmlUrl;
  const releaseNotesUrl = /^https:\/\//.test(notesUrl) ? notesUrl : null;
  if (!releaseNotes && !releaseNotesUrl) return null;
  return { releaseNotes, releaseNotesUrl };
}

// Fetch release notes from a fixed download page whose changelog is the
// `<ul class="version_list">` list inside one platform block (QQ Music's Linux
// block, located by `blockMarker`). Each `<li>` is already a `- …` bullet.
async function fetchChangelogListHtmlReleaseNotes(app, config) {
  let html;
  try {
    html = await fetchText(config.changelogListHtmlUrl);
  } catch (error) {
    log(`  releaseNotes: ${app.applicationId} — ${error.message}`);
    return null;
  }
  const releaseNotes = sanitizeReleaseNotes(
    stripReleaseNotesBoilerplate(
      htmlChangelogToMarkdown(parseHtmlVersionList(html, config.blockMarker)),
      app.applicationId,
    ),
  );
  const notesUrl = config.releaseNotesUrl ?? config.changelogListHtmlUrl;
  const releaseNotesUrl = /^https:\/\//.test(notesUrl) ? notesUrl : null;
  if (!releaseNotes && !releaseNotesUrl) return null;
  return { releaseNotes, releaseNotesUrl };
}

// Fetch release notes from a fixed changelog page whose body is one rich block
// located by `blockMarker` (WPS Linux's `log_main` block, starting at the
// `<h2 class="log_title">` heading). Converted to Markdown via the shared
// HTML→Markdown converter.
async function fetchChangelogBlockHtmlReleaseNotes(app, config) {
  let html;
  try {
    html = await fetchText(config.changelogBlockHtmlUrl);
  } catch (error) {
    log(`  releaseNotes: ${app.applicationId} — ${error.message}`);
    return null;
  }
  const releaseNotes = sanitizeReleaseNotes(
    stripReleaseNotesBoilerplate(extractHtmlBlockToMarkdown(html, config.blockMarker), app.applicationId),
  );
  const notesUrl = config.releaseNotesUrl ?? config.changelogBlockHtmlUrl;
  const releaseNotesUrl = /^https:\/\//.test(notesUrl) ? notesUrl : null;
  if (!releaseNotes && !releaseNotesUrl) return null;
  return { releaseNotes, releaseNotesUrl };
}

// Fetch release notes from a JSONP changelog script (QQ): the `#/log` SPA loads
// `.../rainbow/linuxLog.js` whose payload is `var params = [{version, date,
// feature}, ...]`. Select the entry matching the resolved version, else the
// first (latest), and render its `feature` list as Markdown.
async function fetchChangelogJsonpReleaseNotes(app, config, entry) {
  const version = config.versionField === "version"
    ? entry?.version
    : (entry?.websiteVersion ?? entry?.version);
  if (!version) return null;
  let text;
  try {
    text = await fetchText(config.changelogJsonpUrl);
  } catch (error) {
    log(`  releaseNotes: ${app.applicationId} — ${error.message}`);
    return null;
  }
  const selected = selectJsonpChangelogEntry(parseJsonpChangelog(text), version);
  const releaseNotes = sanitizeReleaseNotes(
    stripReleaseNotesBoilerplate(jsonpEntryToMarkdown(selected), app.applicationId),
  );
  const notesUrl = config.releaseNotesUrl;
  const releaseNotesUrl = typeof notesUrl === "string" && /^https:\/\//.test(notesUrl) ? notesUrl : null;
  if (!releaseNotes && !releaseNotesUrl) return null;
  return { releaseNotes, releaseNotesUrl };
}

async function fetchGitHubReleaseNotes(app, config) {
  let payload;
  try {
    payload = JSON.parse(await fetchText(config.releaseApiUrl, githubApiHeaders()));
  } catch (error) {
    log(`  releaseNotes: ${app.applicationId} — ${error.message}`);
    return null;
  }
  const release = selectReleaseNotesRelease(payload, config.tagPrefix);
  if (!release) return null;
  const releaseNotesUrl = typeof release.html_url === "string" && /^https:\/\//.test(release.html_url)
    ? release.html_url
    : null;
  const releaseNotes = sanitizeReleaseNotes(
    stripReleaseNotesBoilerplate(release.body, app.applicationId),
  );
  if (!releaseNotes && !releaseNotesUrl) return null;
  return { releaseNotes, releaseNotesUrl };
}

async function fetchAtomReleaseNotes(app, config) {
  let xml;
  try {
    xml = await fetchText(config.atomUrl);
  } catch (error) {
    log(`  releaseNotes: ${app.applicationId} — ${error.message}`);
    return null;
  }
  const parsed = atomChangelog(xml, config.titleContains);
  if (!parsed) return null;
  const releaseNotes = sanitizeReleaseNotes(
    stripReleaseNotesBoilerplate(parsed.releaseNotes, app.applicationId),
  );
  const releaseNotesUrl = parsed.releaseNotesUrl;
  if (!releaseNotes && !releaseNotesUrl) return null;
  return { releaseNotes, releaseNotesUrl };
}

async function stableDownloadEntry(app, source) {
  // The version marker is optional: when absent, skip scraping the page and use
  // the .deb control field's Version as the authoritative version (e.g. Bitwarden's
  // version-pinned "latest" URL, which has no server-rendered version text).
  let displayVersion = null;
  if (source.pageVersionMarker) {
    let html;
    try {
      html = await fetchTextFallback(source.officialPageUrl, source.gatewayUrl);
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
    windowFile = await downloadTempFallback(app.applicationId, downloadUrl, source.gatewayUrl);
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
  downloadedDebs.set(label, path);
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

  // Chromium-family browsers (Chrome, Edge) ship their icon as
  // /opt/<vendor>/<product>/product_logo_<size>.png. Electron apps (e.g. Tencent
  // Docs) commonly ship /opt/<app>/resources/icon.png. Only match those narrow,
  // well-known patterns so unrelated PNGs elsewhere under /opt are never picked
  // up (the app's own 1024x1024 icon wins the largest-resolution pick anyway).
  const optBase = join(extractDir, "opt");
  if (existsSync(optBase)) {
    const walkLogos = (dir) => {
      for (const entry of readdirSync(dir, { withFileTypes: true })) {
        const path = join(dir, entry.name);
        if (entry.isDirectory()) walkLogos(path);
        else if (
          /^product_logo_\d+\.png$/i.test(entry.name) ||
          (entry.name.toLowerCase() === "icon.png" && basename(dir) === "resources")
        ) {
          candidates.push(path);
        }
      }
    };
    walkLogos(optBase);
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

// Some vendor endpoints (e.g. Tencent Meeting's query-download-info) require
// a fresh random nonce / request id on every call. A static `query` value would
// be rejected on retries, so `versionEndpoint` supports three dynamic tokens in
// string query values:
//   "{nonce}"     -> 16-char random alphanumeric token
//   "{rnds}"      -> 8-char random alphanumeric token
//   "{timestamp}" -> current epoch millis (string)
function randomAlphanumeric(length) {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
  const bytes = randomBytes(length);
  let output = "";
  for (let index = 0; index < length; index += 1) {
    output += alphabet[bytes[index] % alphabet.length];
  }
  return output;
}

function buildVersionEndpointUrl(source) {
  const url = new URL(source.versionEndpointUrl);
  for (const [key, value] of Object.entries(source.query || {})) {
    let resolved = value;
    if (value === "{nonce}") resolved = randomAlphanumeric(16);
    else if (value === "{rnds}") resolved = randomAlphanumeric(8);
    else if (value === "{timestamp}") resolved = String(Date.now());
    url.searchParams.set(key, typeof resolved === "string" ? resolved : JSON.stringify(resolved));
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
// md5QuerySign: the vendor CDN requires `?t=<epoch-seconds>&k=<hex md5 of
// secret + pathname + t>` (e.g. WPS). `rawUrl` must already be a valid https
// URL; the signature is appended to it in place.
function md5QuerySignUrl(sign, rawUrl) {
  const url = new URL(rawUrl);
  const timestamp = String(Math.floor(Date.now() / 1000));
  const digest = createHash("md5")
    .update(`${sign.secret}${url.pathname}${timestamp}`)
    .digest("hex");
  url.searchParams.set(sign.timestampParam, timestamp);
  url.searchParams.set(sign.signatureParam, digest);
  return url.toString();
}

async function applySign(sign, rawUrl) {
  if (sign.kind === "md5QuerySign") {
    return md5QuerySignUrl(sign, rawUrl);
  }
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

async function toolEntry(tool, versionOverrides) {
  const versionOverride = versionOverrides?.[tool.toolId];
  if (!tool.npmPackage && !versionOverride) {
    fail(tool.toolId, "未配置 npm 包，也没有版本覆盖源");
    return null;
  }

  let version;
  let publishTime = null;
  if (versionOverride) {
    // Non-npm tools (e.g. git/Python installers like Hermes Agent): resolve the
    // latest version from the vendor's GitHub releases, parsing it out of the
    // newest release title (e.g. "Hermes Agent v0.21.0 (v2026.8.31)" -> 0.21.0).
    let payload;
    try {
      payload = JSON.parse(await fetchText(versionOverride.releaseApiUrl, githubApiHeaders()));
    } catch (error) {
      fail(tool.toolId, `读取版本覆盖源失败：${error.message}`);
      return null;
    }
    const releases = Array.isArray(payload) ? payload : [payload];
    const release = releases.find(
      (item) => item && typeof item === "object" && !item.draft && !item.prerelease,
    );
    if (!release) {
      fail(tool.toolId, "版本覆盖源未返回正式发布");
      return null;
    }
    const title = String(release.name ?? release.tag_name ?? "");
    const match = title.match(new RegExp(versionOverride.versionTitleRegex));
    if (!match?.[1]) {
      fail(tool.toolId, `无法从发布标题解析版本：${title}`);
      return null;
    }
    version = match[1];
    publishTime = parseUnixSeconds(release.published_at);
  } else {
    const pkg = tool.npmPackage;
    let doc;
    try {
      doc = JSON.parse(await fetchText(`https://registry.npmjs.org/${encodeURIComponent(pkg)}`));
    } catch (error) {
      fail(tool.toolId, `读取 npm 包信息失败：${error.message}`);
      return null;
    }
    // tools may pin the npm dist-tag channel they track (e.g. dsh tracks
    // `alpha`); an unconfigured tool follows npm's `latest` tag.
    try {
      version = resolveNpmDistTagVersion(doc, tool.distTag);
    } catch (error) {
      fail(tool.toolId, `npm ${pkg}：${error.message}`);
      return null;
    }
    publishTime = parseUnixSeconds(doc?.time?.[version]);
  }

  const entry = {
    version: String(version),
  };
  // Omit `npmPackage` for tools distributed outside npm (e.g. Hermes Agent)
  // instead of emitting `null`. `FeedToolEntry.npm_package` is optional
  // (`#[serde(default)]`), so omitting the key is the canonical representation;
  // older app versions that require a string still can't parse such entries,
  // so non-npm tools effectively need a recent app version.
  if (tool.npmPackage) {
    entry.npmPackage = `${tool.npmPackage}`;
  }
  entry._versionTimeCandidate = publishTime != null
    ? { time: publishTime, source: "official" }
    : null;
  return entry;
}

// Fetch a dev tool's changelog from either a fixed CHANGELOG.md-style Markdown
// URL (extracting the section for the resolved version) or a GitHub Releases
// endpoint (selecting the release whose tag matches the version). CI-only
// config lives in `toolReleaseNotesOverrides` keyed by tool id. Best-effort: a
// failure leaves the note absent, never drops the tool entry.
async function fetchToolReleaseNotes(tool, config, entry) {
  if (!config || typeof config !== "object" || !entry?.version) return null;
  if (typeof config.changelogMarkdownUrl === "string") return fetchToolMarkdownReleaseNotes(tool, config, entry);
  if (typeof config.releaseApiUrl === "string") return fetchToolGitHubReleaseNotes(tool, config, entry);
  return null;
}

async function fetchToolMarkdownReleaseNotes(tool, config, entry) {
  let text;
  try {
    text = await fetchText(config.changelogMarkdownUrl);
  } catch (error) {
    log(`  toolReleaseNotes: ${tool.toolId} — ${error.message}`);
    return null;
  }
  const section = extractMarkdownVersionSection(text, entry.version);
  const releaseNotes = sanitizeReleaseNotes(
    stripReleaseNotesBoilerplate(cleanReleaseNotesMarkdown(section), tool.toolId),
  );
  const releaseNotesUrl = typeof config.releaseNotesUrl === "string" && /^https:\/\//.test(config.releaseNotesUrl)
    ? config.releaseNotesUrl
    : null;
  if (!releaseNotes && !releaseNotesUrl) return null;
  return { releaseNotes, releaseNotesUrl };
}

async function fetchToolGitHubReleaseNotes(tool, config, entry) {
  let payload;
  try {
    payload = JSON.parse(await fetchText(config.releaseApiUrl, githubApiHeaders()));
  } catch (error) {
    log(`  toolReleaseNotes: ${tool.toolId} — ${error.message}`);
    return null;
  }
  const release = selectToolRelease(payload, config.tagPrefix, entry.version);
  if (!release) return null;
  const releaseNotesUrl = typeof release.html_url === "string" && /^https:\/\//.test(release.html_url)
    ? release.html_url
    : null;
  const releaseNotes = sanitizeReleaseNotes(
    stripReleaseNotesBoilerplate(release.body, tool.toolId),
  );
  if (!releaseNotes && !releaseNotesUrl) return null;
  return { releaseNotes, releaseNotesUrl };
}

// ---------------------------------------------------------------------------
// Config loading / source-group helpers
// ---------------------------------------------------------------------------

function loadConfig() {
  const catalog = JSON.parse(readFileSync(CATALOG_PATH, "utf8"));

  let extra = { applications: [], sources: {} };
  try {
    extra = JSON.parse(readFileSync(EXTRA_CATALOG_PATH, "utf8"));
  } catch (error) {
    if (error.code !== "ENOENT") {
      fail("feed-sources.json", `解析额外软件源失败：${error.message}`);
    }
  }
  const extraApplications = extra.applications || [];
  const sourceRegistry = extra.sources || {};
  const releaseNotesOverrides = extra.releaseNotesOverrides || {};
  const toolReleaseNotesOverrides = extra.toolReleaseNotesOverrides || {};
  const toolVersionOverrides = extra.toolVersionOverrides || {};

  const categoryConfig = readCategoryConfig();
  const categories = (categoryConfig.categories || [])
    .map((category) => ({ id: String(category?.id ?? ""), label: String(category?.label ?? category?.id ?? "") }))
    .filter((category) => category.id !== "");
  const categoryAssignments = {
    applications: categoryConfig.assignments?.applications ?? {},
    developmentTools: categoryConfig.assignments?.developmentTools ?? {},
  };

  return {
    catalog,
    extraApplications,
    sourceRegistry,
    releaseNotesOverrides,
    toolReleaseNotesOverrides,
    toolVersionOverrides,
    categories,
    categoryAssignments,
  };
}

function logCategoryNotes(catalog, extraApplications) {
  const { categories, assignments } = readCategoryConfig();
  const knownCategoryIds = new Set((categories || []).map((category) => String(category?.id ?? "")).filter(Boolean));
  const appAssignments = assignments?.applications ?? {};
  const toolAssignments = assignments?.developmentTools ?? {};
  const missingAssignments = [];
  for (const app of [...(catalog.applications || []), ...extraApplications]) {
    if (!appAssignments[app.applicationId]) missingAssignments.push(`应用 ${app.applicationId}`);
  }
  for (const tool of catalog.developmentTools || []) {
    if (!toolAssignments[tool.toolId]) missingAssignments.push(`工具 ${tool.toolId}`);
  }
  if (missingAssignments.length > 0) log(`分类缺失：${missingAssignments.join("、")} 将落入「其他」`);
  const unknownCategoryIds = [
    ...Object.entries(appAssignments).filter(([, id]) => !knownCategoryIds.has(id)).map(([appId, id]) => `应用 ${appId}->${id}`),
    ...Object.entries(toolAssignments).filter(([, id]) => !knownCategoryIds.has(id)).map(([toolId, id]) => `工具 ${toolId}->${id}`),
  ];
  if (unknownCategoryIds.length > 0) log(`未知分类 id：${unknownCategoryIds.join("、")}`);
}

// Discover source feeds under <partsDir>/<group>/feed.<group>.json. A group
// whose directory exists but carries no feed (its generate job failed or was
// cancelled before upload) is simply absent from the result; the merge then
// falls back to the previous central feed for everything that group owns.
function discoverSourceFeeds(partsDir) {
  const sourceFeeds = {};
  if (!existsSync(partsDir)) return sourceFeeds;
  for (const entry of readdirSync(partsDir, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    const file = join(partsDir, entry.name, `feed.${entry.name}.json`);
    if (existsSync(file)) {
      try {
        sourceFeeds[entry.name] = JSON.parse(readFileSync(file, "utf8"));
      } catch (error) {
        fail(`源 feed ${entry.name}`, `解析失败：${error.message}`);
      }
    }
  }
  return sourceFeeds;
}

// ---------------------------------------------------------------------------
// Entry scraping (shared by full + partial modes)
// ---------------------------------------------------------------------------

async function scrapeAppEntry(app, releaseNotesOverrides) {
  const kind = app.source?.kind;
  let entry = null;
  if (kind === "aptRepository") {
    entry = await aptEntry(app);
  } else if (kind === "releaseApi") {
    entry = await releaseApiEntry(app, app.source);
  } else if (kind === "stableDownloadEndpoint") {
    entry = await stableDownloadEntry(app, app.source);
  } else if (kind === "versionEndpoint") {
    entry = await versionEndpointEntry(app, app.source);
  }
  // browserImport and unknown kinds yield no entry (previous-feed fallback handles it).
  // Non-releaseApi sources may still publish a changelog (GitHub release, Atom
  // feed, or a versioned HTML updates page); attach it best-effort.
  // releaseApiEntry already set these, so only fill gaps. Built-in apps whose
  // changelog config is CI-only live in `releaseNotesOverrides` keyed by id.
  const releaseNotesConfig = app.releaseNotes ?? releaseNotesOverrides[app.applicationId];
  if (entry && entry.releaseNotes == null && entry.releaseNotesUrl == null && releaseNotesConfig) {
    const notes = await fetchReleaseNotes(app, releaseNotesConfig, entry);
    if (notes) {
      entry.releaseNotes = notes.releaseNotes;
      entry.releaseNotesUrl = notes.releaseNotesUrl;
    }
  }
  return entry;
}

// ---------------------------------------------------------------------------
// Catalog / icon helpers
// ---------------------------------------------------------------------------

// A CI-only application record destined for the signed catalog: strip fields
// the desktop app / helper must never see (private gateway endpoint,
// release-notes fetch config, CI-only source group).
function toCatalogApplication(app) {
  const record = structuredClone(app);
  if (record.source && "gatewayUrl" in record.source) delete record.source.gatewayUrl;
  if ("releaseNotes" in record) delete record.releaseNotes;
  if ("sourceGroup" in record) delete record.sourceGroup;
  return record;
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

// Extract + publish icons for feed-added applications and inject iconUrl /
// iconSha256 into their catalog records. Built-in apps keep their bundled
// icons. Two optimizations over the original single-pass run:
//   - the vendor .deb downloaded for the entry is reused (downloadedDebs)
//     instead of being downloaded a second time;
//   - when the resolved entry is byte-identical to the previous feed's
//     (same sha256) and the previous catalog already carried the icon, the
//     download is skipped entirely — the old icon is provably still correct.
async function buildCatalogRecords({ extraApps, applications, previousFeed, iconsDir, iconBase }) {
  const records = {};
  for (const app of extraApps) {
    records[app.applicationId] = toCatalogApplication(app);
  }
  if (!iconBase) return records;

  mkdirSync(iconsDir, { recursive: true });
  const previousCatalogMap = parseCatalogApplications(previousFeed?.catalogJson);
  for (const app of extraApps) {
    const record = records[app.applicationId];
    const entry = applications[app.applicationId];
    if (!entry?.downloadUrl) {
      log(`  icon: ${app.applicationId} — 无下载地址，跳过`);
      continue;
    }
    const previousApp = previousFeed?.applications?.[app.applicationId];
    const previousIcon = previousCatalogMap?.[app.applicationId];
    if (previousApp && previousIcon?.iconUrl && previousIcon?.iconSha256 && entry.sha256 === previousApp.sha256) {
      record.iconUrl = previousIcon.iconUrl;
      record.iconSha256 = previousIcon.iconSha256;
      log(`  icon: ${app.applicationId} — 版本未变，沿用上一版图标`);
      continue;
    }
    const freshDeb = downloadedDebs.get(app.applicationId);
    let debPath = freshDeb;
    let downloadedHere = false;
    if (!debPath) {
      try {
        debPath = await downloadTemp(app.applicationId, await iconDownloadUrl(app, entry));
        downloadedHere = true;
      } catch (error) {
        fail(app.applicationId, `图标下载失败：${error.message}`);
        continue;
      }
    }
    try {
      const icon = extractIcon(debPath);
      if (!icon) {
        log(`  icon: ${app.applicationId} — 未找到图标`);
        continue;
      }
      const iconPath = join(iconsDir, `${app.applicationId}.png`);
      writeFileSync(iconPath, icon.buffer);
      // catalogJson uses camelCase to match the serde Application model.
      record.iconUrl = `${iconBase}/icons/${app.applicationId}.png`;
      record.iconSha256 = sha256Buf(icon.buffer);
      log(`  icon: ${app.applicationId} ✓ (${icon.width}x${icon.height})`);
    } catch (error) {
      fail(app.applicationId, `图标提取失败：${error.message}`);
    } finally {
      if (downloadedHere) rmSync(debPath, { force: true });
    }
  }
  return records;
}

// ---------------------------------------------------------------------------
// Output writers
// ---------------------------------------------------------------------------

function writeSignedOutput(OUT_PATH, feed) {
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
}

function logReused(reusedEntries) {
  if (reusedEntries.length > 0) {
    log(`\n${reusedEntries.length} source(s) failed and reused the previous feed entry:`);
    for (const e of reusedEntries) log(`  - ${e}`);
  }
}

function logErrors(errorList) {
  if (errorList.length > 0) {
    log(`\n${errorList.length} source(s) were skipped and will be unavailable in the feed:`);
    for (const e of errorList) log(`  - ${e}`);
  }
}

// ---------------------------------------------------------------------------
// Generation modes
// ---------------------------------------------------------------------------

// Scrape the central-only data (selfUpdate + development tools) with
// previous-feed fallback and version-time merging. Used by full generation
// and by the merge step — source feeds never carry these (they are
// central-only per DESIGN-multi-source.md §4.2).
async function scrapeCentralData(config, previousFeed, nowUnixSeconds, reusedEntries) {
  const { catalog, toolReleaseNotesOverrides, toolVersionOverrides } = config;
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
    if (!selfUpdate && previousFeed?.selfUpdate) {
      selfUpdate = { ...previousFeed.selfUpdate };
      reusedEntries.push("selfUpdate");
    }
  }
  if (selfUpdate) applyVersionTime(selfUpdate, previousFeed?.selfUpdate, nowUnixSeconds);

  const developmentTools = {};
  const tools = catalog.developmentTools || [];
  const scrapedTools = await mapLimit(tools, CONCURRENCY, async (tool) => {
    const entry = await toolEntry(tool, toolVersionOverrides);
    // Attach a changelog for tools configured in `toolReleaseNotesOverrides`.
    if (entry && entry.releaseNotes == null && entry.releaseNotesUrl == null) {
      const notes = await fetchToolReleaseNotes(tool, toolReleaseNotesOverrides[tool.toolId], entry);
      if (notes) {
        entry.releaseNotes = notes.releaseNotes;
        entry.releaseNotesUrl = notes.releaseNotesUrl;
      }
    }
    return entry;
  });
  for (let index = 0; index < tools.length; index += 1) {
    const tool = tools[index];
    const resolved = entryOrPrevious(scrapedTools[index], previousFeed?.developmentTools?.[tool.toolId]);
    if (resolved) {
      developmentTools[tool.toolId] = resolved;
      if (!scrapedTools[index]) reusedEntries.push(`工具 ${tool.toolId}`);
    }
    if (resolved) applyVersionTime(resolved, previousFeed?.developmentTools?.[tool.toolId], nowUnixSeconds);
  }
  return { selfUpdate, developmentTools };
}

// Generation: app entries + icons + catalog for either the central feed
// (group == null, full single pass, local `npm run update-feed`) or one source
// group (group == <id>, producing that group's final signed source feed).
// Every mode does the never-silently-drop fallback and version-time merge
// inline, with bounded concurrency and .deb reuse.
async function runGenerate({ OUT_PATH, config, previousFeed, group = null }) {
  const {
    catalog,
    extraApplications,
    releaseNotesOverrides,
    categories,
    categoryAssignments,
  } = config;
  const nowUnixSeconds = Math.floor(Date.now() / 1000);
  const reusedEntries = [];
  const isGroup = group !== null;
  const groupApps = [...(catalog.applications || []), ...extraApplications].filter(
    (app) => !isGroup || sourceGroupOf(app) === group,
  );
  const groupExtra = extraApplications.filter((app) => !isGroup || sourceGroupOf(app) === group);

  const applications = {};
  const scraped = await mapLimit(groupApps, CONCURRENCY, (app) => scrapeAppEntry(app, releaseNotesOverrides));
  for (let index = 0; index < groupApps.length; index += 1) {
    const app = groupApps[index];
    const entry = scraped[index];
    const resolved = entryOrPrevious(entry, previousFeed?.applications?.[app.applicationId]);
    if (resolved) {
      applications[app.applicationId] = resolved;
      if (!entry) reusedEntries.push(`应用 ${app.applicationId}`);
    }
  }
  for (const [id, entry] of Object.entries(applications)) {
    applyVersionTime(entry, previousFeed?.applications?.[id], nowUnixSeconds);
  }

  const iconBase = catalog.metadataFeed?.url?.replace(/\/[^/]*$/, "");
  const iconsDir = join(dirname(OUT_PATH), "icons");
  const catalogRecords = await buildCatalogRecords({
    extraApps: groupExtra,
    applications,
    previousFeed,
    iconsDir,
    iconBase,
  });
  const catalogList = groupExtra.map((app) => catalogRecords[app.applicationId]).filter(Boolean);
  const catalogJson = JSON.stringify(catalogList);
  const catalogSignature = process.env.FEED_SIGNING_KEY
    ? sign(null, Buffer.from(catalogJson), process.env.FEED_SIGNING_KEY).toString("hex")
    : null;

  const feed = {
    schemaVersion: 2,
    generatedAtUnixSeconds: nowUnixSeconds,
    applications,
    catalogJson,
    catalogSignature,
  };
  if (!isGroup) {
    // Central feed only: selfUpdate, development tools and categories live
    // exclusively in the central source; source feeds are apps-only.
    const { selfUpdate, developmentTools } = await scrapeCentralData(config, previousFeed, nowUnixSeconds, reusedEntries);
    feed.selfUpdate = selfUpdate;
    feed.developmentTools = developmentTools;
    feed.categories = categories;
    feed.categoryAssignments = categoryAssignments;
  }

  writeSignedOutput(OUT_PATH, feed);
  if (isGroup) {
    log(`  apps=${Object.keys(applications).length} catalog=${catalogList.length}`);
  } else {
    log(
      `applications=${Object.keys(applications).length} extraCatalog=${catalogList.length} selfUpdate=${feed.selfUpdate !== null} tools=${Object.keys(feed.developmentTools || {}).length}`,
    );
  }
  logReused(reusedEntries);
  logErrors(errors);
}

// Merge mode: aggregate every discovered source feed into the central
// feed.json. Source feeds are final (their group job already did fallback +
// version-time merge), so this step unions them in canonical app order,
// falls back to the previous central feed for a missing group, scrapes the
// central-only selfUpdate + development tools, assembles + signs the full
// catalog, and copies each source feed (+ its signature) into the output dir
// so Pages hosts feed.tencent.json / feed.common.json alongside feed.json.
async function runMerge({ OUT_PATH, partsDir, config, previousFeed }) {
  const {
    catalog,
    extraApplications,
    categories,
    categoryAssignments,
  } = config;
  const nowUnixSeconds = Math.floor(Date.now() / 1000);
  const sourceFeeds = discoverSourceFeeds(partsDir);
  for (const [group, feed] of Object.entries(sourceFeeds)) {
    log(`  发现源 feed：${group}（apps=${Object.keys(feed.applications || {}).length}）`);
  }

  const merged = mergeSourceFeeds({
    allApps: [...(catalog.applications || []), ...extraApplications],
    extraApps: extraApplications,
    sourceFeeds,
    previousFeed,
    builtinGroups: SOURCE_GROUPS,
  });
  const { applications, catalogApps } = merged;
  for (const group of merged.missingGroups) {
    log(`!! 源组 ${group} 未产出 feed，该组应用回落上一版条目`);
  }

  // Icons: prefer the group artifacts; fall back to the previous Pages deploy.
  const iconBase = catalog.metadataFeed?.url?.replace(/\/[^/]*$/, "");
  const iconsDir = join(dirname(OUT_PATH), "icons");
  if (iconBase) {
    mkdirSync(iconsDir, { recursive: true });
    for (const [appId, record] of Object.entries(catalogApps)) {
      if (!record?.iconUrl) continue;
      const target = join(iconsDir, `${appId}.png`);
      if (existsSync(target)) continue;
      let copied = false;
      for (const group of Object.keys(sourceFeeds)) {
        const candidate = join(partsDir, group, "icons", `${appId}.png`);
        if (existsSync(candidate)) {
          copyFileSync(candidate, target);
          copied = true;
          break;
        }
      }
      if (copied) continue;
      try {
        const buffer = await fetchBuffer(record.iconUrl);
        writeFileSync(target, buffer);
        log(`  icon: ${appId} — 从上一版页面补取`);
      } catch (error) {
        fail(appId, `图标补取失败：${error.message}`);
      }
    }
  }

  // Central-only data: selfUpdate + development tools are scraped here (a few
  // fast GitHub/npm calls), with previous-feed fallback + version-time merge.
  const reusedEntries = [];
  const { selfUpdate, developmentTools } = await scrapeCentralData(config, previousFeed, nowUnixSeconds, reusedEntries);

  const catalogList = extraApplications.map((app) => catalogApps[app.applicationId]).filter(Boolean);
  const catalogJson = JSON.stringify(catalogList);
  const catalogSignature = process.env.FEED_SIGNING_KEY
    ? sign(null, Buffer.from(catalogJson), process.env.FEED_SIGNING_KEY).toString("hex")
    : null;

  const feed = {
    schemaVersion: 2,
    generatedAtUnixSeconds: nowUnixSeconds,
    applications,
    catalogJson,
    catalogSignature,
    selfUpdate,
    developmentTools,
    categories,
    categoryAssignments,
  };

  // Publish the per-source feeds alongside the central feed: copy each source
  // feed + signature into the output directory so upload-pages deploys them.
  for (const group of Object.keys(sourceFeeds)) {
    const src = join(partsDir, group, `feed.${group}.json`);
    if (!existsSync(src)) continue;
    const dstDir = dirname(OUT_PATH);
    const dst = join(dstDir, `feed.${group}.json`);
    copyFileSync(src, dst);
    const sig = `${src}.sig`;
    if (existsSync(sig)) copyFileSync(sig, `${dst}.sig`);
    log(`  已复制 ${group} 源 feed 到发布目录`);
  }

  writeSignedOutput(OUT_PATH, feed);
  log(
    `applications=${Object.keys(applications).length} extraCatalog=${catalogList.length} selfUpdate=${selfUpdate !== null} tools=${Object.keys(developmentTools).length}`,
  );
  logReused([...merged.reused, ...reusedEntries]);
  for (const note of merged.notes) log(`  ${note}`);
  logErrors(errors);
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.merge && args.group) {
    throw new Error("--merge 不能与 --group 同时使用");
  }
  if (args.merge && !args.parts) {
    throw new Error("--merge 需要 --parts <dir>");
  }

  const config = loadConfig();
  const knownGroups = [...new Set([...Object.keys(config.sourceRegistry), ...Object.keys(SOURCE_GROUPS), DEFAULT_SOURCE_GROUP])];
  if (args.group && !knownGroups.includes(args.group)) {
    throw new Error(`未知源组：${args.group}（可用：${knownGroups.join("、")}）`);
  }
  for (const problem of validateSourceGroups([...(config.catalog.applications || []), ...config.extraApplications], SOURCE_GROUPS, knownGroups)) {
    fail("sourceGroup", problem);
  }
  logCategoryNotes(config.catalog, config.extraApplications);

  const OUT_PATH = args.out ?? args.positional ?? resolve(REPO_ROOT, "dist", "feed.json");

  // The previous published feed is fetched up front in every mode: it is the
  // state reference for version-update-time merging, the fallback when a
  // scrape fails, and — in generation mode — the reference for skipping
  // unchanged-icon re-downloads. Never silently drop an app that was in the
  // previous feed just because a single scrape failed.
  const previousFeed = await fetchPreviousFeed(config.catalog.metadataFeed?.url);

  if (args.merge) {
    await runMerge({ OUT_PATH, partsDir: args.parts, config, previousFeed });
    return;
  }
  if (args.group) {
    await runGenerate({ OUT_PATH, config, previousFeed, group: args.group });
    return;
  }
  await runGenerate({ OUT_PATH, config, previousFeed });
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
