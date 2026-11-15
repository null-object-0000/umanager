import fs from "node:fs";
import path from "node:path";
import { createHash, createPublicKey, verify } from "node:crypto";
import { fileURLToPath } from "node:url";

const canonicalBase = "https://null-object-0000.github.io/umanager/";
const hash = bytes => createHash("sha256").update(bytes).digest("hex");
function verifyBytes(bytes, signature, publicKey) {
  if (!/^[a-f0-9]{128}$/i.test(signature) || !/^[a-f0-9]{64}$/i.test(publicKey)) {
    throw new Error("Invalid Ed25519 signature or public key encoding");
  }
  const key = createPublicKey({
    key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), Buffer.from(publicKey, "hex")]),
    format: "der", type: "spki",
  });
  if (!verify(null, bytes, key, Buffer.from(signature, "hex"))) throw new Error("Invalid publication signature");
}

export function centralPublicKey() {
  const source = fs.readFileSync(new URL("../../src-tauri/src/feed.rs", import.meta.url), "utf8");
  const key = source.match(/const FEED_PUBLIC_KEY_HEX: &str = "([a-f0-9]{64})";/)?.[1];
  if (!key) throw new Error("Cannot read the existing client public key");
  return key;
}

export function prepareMirror(sourceDirectory, outputDirectory, { publicKey = centralPublicKey(), sourceRunId = "local" } = {}) {
  const source = path.resolve(sourceDirectory), output = path.resolve(outputDirectory);
  if (output === source || output.startsWith(source + path.sep) || source.startsWith(output + path.sep)) {
    throw new Error("Publication input and output must be separate directories");
  }
  if (fs.existsSync(output) && fs.readdirSync(output).length) throw new Error("Mirror output must be empty");
  const files = new Map();
  function walk(directory, prefix = "") {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const relative = prefix + entry.name;
      if (entry.isSymbolicLink()) throw new Error("Publication symlinks are forbidden");
      if (entry.isDirectory()) walk(path.join(directory, entry.name), relative + "/");
      else if (entry.isFile()) {
        if (relative === ".nojekyll") continue;
        if (!/^(?:v3\/)?(?:feed(?:\.[a-z0-9-]+)?\.json(?:\.sig)?|icons\/[a-zA-Z0-9._-]+\.png)$/.test(relative)) {
          throw new Error("Unexpected publication file: " + relative);
        }
        files.set(relative, fs.readFileSync(path.join(directory, entry.name)));
      } else throw new Error("Publication contains a special file");
    }
  }
  walk(source);
  const required = relative => { const bytes = files.get(relative); if (!bytes) throw new Error("Missing publication file: " + relative); return bytes; };
  const parsed = new Map();
  function verifyFeed(relative, key) {
    const bytes = required(relative), signature = required(relative + ".sig").toString("utf8").trim();
    if (bytes.length > 1024 * 1024) throw new Error("Feed exceeds the existing client size limit");
    verifyBytes(bytes, signature, key);
    const feed = JSON.parse(bytes.toString("utf8"));
    if (![2, 3].includes(feed.schemaVersion)) throw new Error("Unsupported signed feed schema");
    if (feed.catalogJson !== undefined || feed.catalogSignature !== undefined) {
      if (typeof feed.catalogJson !== "string" || typeof feed.catalogSignature !== "string") throw new Error("Incomplete signed catalog");
      verifyBytes(Buffer.from(feed.catalogJson), feed.catalogSignature, key);
      for (const app of JSON.parse(feed.catalogJson)) {
        if (!app.iconUrl) continue;
        if (!app.iconUrl.startsWith(canonicalBase)) throw new Error("Unexpected publication icon origin");
        const iconPath = app.iconUrl.slice(canonicalBase.length);
        if (hash(required(iconPath)) !== app.iconSha256?.toLowerCase()) throw new Error("Invalid signed icon digest: " + iconPath);
      }
    }
    parsed.set(relative, feed);
    return feed;
  }
  verifyFeed("feed.json", publicKey);
  const central = verifyFeed("v3/feed.json", publicKey);
  const sourceKeys = new Map();
  for (const entry of central.sources ?? []) {
    if (!/^[a-z0-9-]+$/.test(entry.id) || sourceKeys.has(entry.id)) throw new Error("Invalid source identity");
    const expectedUrl = canonicalBase + `v3/feed.${entry.id}.json`;
    if (entry.url !== expectedUrl) throw new Error("Signed source identity changed");
    verifyBytes(Buffer.from(JSON.stringify({ sourceId: entry.id, feedUrl: entry.url, publicKeyHex: entry.publicKeyHex })), entry.endorsement, publicKey);
    sourceKeys.set(entry.id, entry.publicKeyHex);
  }
  for (const relative of files.keys()) {
    if (!relative.endsWith(".json") || parsed.has(relative)) continue;
    const match = relative.match(/^v3\/feed\.([a-z0-9-]+)\.json$/);
    const key = match ? sourceKeys.get(match[1]) : publicKey;
    if (!key) throw new Error("Source feed is not endorsed by the central feed");
    const feed = verifyFeed(relative, key);
    if (match && (feed.source?.id !== match[1] || feed.source?.url !== canonicalBase + relative || feed.source?.publicKeyHex !== key)) {
      throw new Error("Source feed self-description differs from its signed identity");
    }
  }
  for (const id of sourceKeys.keys()) if (!parsed.has(`v3/feed.${id}.json`)) throw new Error("Missing endorsed source feed");
  for (const relative of files.keys()) if (relative.endsWith(".sig") && !parsed.has(relative.slice(0, -4))) throw new Error("Orphan signature");
  fs.mkdirSync(output, { recursive: true });
  const manifest = [];
  for (const [relative, bytes] of files) {
    const target = path.join(output, "umanager", relative);
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.writeFileSync(target, bytes);
    manifest.push({ path: "umanager/" + relative, bytes: bytes.length, sha256: hash(bytes) });
  }
  manifest.sort((a, b) => a.path.localeCompare(b.path));
  const report = { sourceRunId: String(sourceRunId), signedFeedsVerified: parsed.size, files: manifest };
  fs.writeFileSync(path.join(output, "mirror-status.json"), JSON.stringify(report, null, 2));
  fs.writeFileSync(path.join(output, "index.html"), '<!doctype html><meta charset="utf-8"><title>UManager update mirror</title><p>UManager signed update metadata mirror. <a href="/umanager/v3/feed.json">Feed</a></p>');
  fs.writeFileSync(path.join(output, "edgeone.json"), JSON.stringify({ headers: [{ source: "/umanager/*", headers: [{ key: "Cache-Control", value: "public,max-age=0,must-revalidate" }] }] }));
  return report;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [, , input, output] = process.argv;
  if (!input || !output) throw new Error("Usage: node prepare.mjs SOURCE OUTPUT");
  const report = prepareMirror(input, output, { sourceRunId: process.env.SOURCE_RUN_ID ?? "local" });
  console.log(JSON.stringify({ sourceRunId: report.sourceRunId, signedFeedsVerified: report.signedFeedsVerified, files: report.files.length }));
}
