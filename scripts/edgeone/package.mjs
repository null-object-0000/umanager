import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import { centralPublicKey, verifyBytes } from "./prepare.mjs";

const maxBytes = 25 * 1024 * 1024;
const hosts = new Set(["github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com"]);
export async function stagePackage(directory, { publicKey = centralPublicKey(), fetchImpl = fetch } = {}) {
  const feedPath = path.join(directory, "umanager/v3/feed.json");
  const bytes = fs.readFileSync(feedPath);
  verifyBytes(bytes, fs.readFileSync(feedPath + ".sig", "utf8").trim(), publicKey);
  const update = JSON.parse(bytes).selfUpdate;
  if (!update || !/^\d+\.\d+\.\d+$/.test(update.version) || update.packageName !== "u-manager" || update.architecture !== "amd64"
      || !Number.isSafeInteger(update.size) || update.size <= 0 || update.size > maxBytes || !/^[a-f0-9]{64}$/i.test(update.sha256)) {
    throw new Error("Unsupported signed UManager package or exceeds the free 25 MiB file limit");
  }
  const asset = `UManager_${update.version}_amd64.deb`, tag = `v${update.version}`;
  if (update.assetName !== asset || update.releaseTag !== tag || update.downloadUrl !== `https://github.com/null-object-0000/umanager/releases/download/${tag}/${asset}`) {
    throw new Error("Unexpected canonical UManager release asset");
  }
  const signal = AbortSignal.timeout(120000);
  let url = new URL(update.downloadUrl), response;
  for (let redirects = 0; ; redirects++) {
    if (url.protocol !== "https:" || (url.port && url.port !== "443") || !hosts.has(url.hostname) || url.username || url.password || url.hash) throw new Error("Untrusted package redirect");
    response = await fetchImpl(url, { redirect: "manual", signal });
    if (response.status >= 300 && response.status < 400) {
      await response.body?.cancel();
      if (redirects >= 5 || !response.headers.get("location")) throw new Error("Invalid package redirect chain");
      url = new URL(response.headers.get("location"), url);
    } else break;
  }
  if (response.status !== 200) { await response.body?.cancel(); throw new Error("Package HTTP " + response.status); }
  const length = response.headers.get("content-length");
  if (length !== null && Number(length) !== update.size) { await response.body?.cancel(); throw new Error("Package response size differs from signed feed"); }
  const chunks = [], hash = createHash("sha256");
  let size = 0;
  for await (const chunk of response.body) {
    size += chunk.length;
    if (size > update.size) { throw new Error("Package exceeds signed size"); }
    chunks.push(chunk); hash.update(chunk);
  }
  if (size !== update.size || hash.digest("hex") !== update.sha256.toLowerCase()) throw new Error("Package size or SHA256 differs from signed feed");
  // Publish no bytes until the entire signed digest and size have been checked.
  const relative = `umanager/releases/${tag}/${asset}`;
  const target = path.join(directory, relative);
  fs.mkdirSync(path.dirname(target), { recursive: true });
  const temporary = target + ".tmp";
  try {
    fs.writeFileSync(temporary, Buffer.concat(chunks), { flag: "wx" });
    fs.renameSync(temporary, target);
  } finally { if (fs.existsSync(temporary)) fs.unlinkSync(temporary); }
  const report = { path: relative, version: update.version, bytes: size, sha256: update.sha256.toLowerCase(), canonicalUrl: update.downloadUrl };
  fs.writeFileSync(path.join(directory, "package-status.json"), JSON.stringify(report, null, 2));
  return report;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (!process.argv[2]) throw new Error("Usage: node package.mjs MIRROR_DIRECTORY");
  console.log(JSON.stringify(await stagePackage(process.argv[2])));
}
