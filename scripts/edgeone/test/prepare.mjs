import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { generateKeyPairSync, sign, createHash } from "node:crypto";
import { prepareMirror } from "../prepare.mjs";
import { resolvePublication } from "../resolve-run.mjs";

const base = "https://null-object-0000.github.io/umanager/";
function fixture(t) {
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "umanager-mirror-test-"));
  t.after(() => {
    if (!path.resolve(temporary).startsWith(path.resolve(os.tmpdir()) + path.sep + "umanager-mirror-test-")) throw new Error("Unsafe test cleanup target");
    fs.rmSync(temporary, { recursive: true, force: true });
  });
  const source = path.join(temporary, "source"), output = path.join(temporary, "mirror");
  fs.mkdirSync(source);
  const central = generateKeyPairSync("ed25519"), group = generateKeyPairSync("ed25519");
  const publicHex = key => key.publicKey.export({ format: "der", type: "spki" }).subarray(-32).toString("hex");
  const publicKey = publicHex(central), groupKey = publicHex(group);
  function write(relative, value) { const file = path.join(source, relative); fs.mkdirSync(path.dirname(file), { recursive: true }); fs.writeFileSync(file, value); }
  function signed(relative, feed, key = central) { const bytes = Buffer.from(JSON.stringify(feed)); write(relative, bytes); write(relative + ".sig", sign(null, bytes, key.privateKey).toString("hex")); }
  const png = Buffer.from("test-icon-bytes");
  write("icons/qq.png", png);
  const catalogJson = JSON.stringify([{ applicationId: "qq", iconUrl: base + "icons/qq.png", iconSha256: createHash("sha256").update(png).digest("hex") }]);
  const feed = { schemaVersion: 2, generatedAtUnixSeconds: 1, applications: {}, catalogJson, catalogSignature: sign(null, Buffer.from(catalogJson), central.privateKey).toString("hex") };
  signed("feed.json", feed);
  signed("feed.tencent.json", feed);
  const entry = { id: "tencent", url: base + "v3/feed.tencent.json", publicKeyHex: groupKey };
  entry.endorsement = sign(null, Buffer.from(JSON.stringify({ sourceId: entry.id, feedUrl: entry.url, publicKeyHex: entry.publicKeyHex })), central.privateKey).toString("hex");
  signed("v3/feed.json", { schemaVersion: 3, generatedAtUnixSeconds: 1, applications: {}, sources: [entry] });
  signed("v3/feed.tencent.json", { schemaVersion: 3, generatedAtUnixSeconds: 1, applications: {}, source: { id: entry.id, url: entry.url, publicKeyHex: groupKey } }, group);
  return { source, output, publicKey, signed, write, central, group, entry };
}

test("publication copies exact signed bytes and keeps canonical source identities", t => {
  const f = fixture(t), report = prepareMirror(f.source, f.output, { publicKey: f.publicKey, sourceRunId: "123" });
  assert.equal(report.signedFeedsVerified, 4);
  for (const file of report.files) assert.deepEqual(fs.readFileSync(path.join(f.output, file.path)), fs.readFileSync(path.join(f.source, file.path.slice("umanager/".length))));
  const source = JSON.parse(fs.readFileSync(path.join(f.output, "umanager/v3/feed.tencent.json")));
  assert.equal(source.source.url, f.entry.url);
  assert.equal(JSON.parse(fs.readFileSync(path.join(f.output, "edgeone.json"))).rewrites, undefined);
});

test("rejects a changed JSON byte even when the JSON still parses", t => {
  const f = fixture(t); fs.appendFileSync(path.join(f.source, "v3/feed.tencent.json"), " ");
  assert.throws(() => prepareMirror(f.source, f.output, { publicKey: f.publicKey }), /Invalid publication signature/);
  assert.equal(fs.existsSync(f.output), false);
});

test("rejects a valid source signature made with the wrong source key", t => {
  const f = fixture(t), data = JSON.parse(fs.readFileSync(path.join(f.source, "v3/feed.tencent.json")));
  f.signed("v3/feed.tencent.json", data, f.central);
  assert.throws(() => prepareMirror(f.source, f.output, { publicKey: f.publicKey }), /Invalid publication signature/);
});

test("rejects an invalid central source endorsement", t => {
  const f = fixture(t), data = JSON.parse(fs.readFileSync(path.join(f.source, "v3/feed.json")));
  data.sources[0].endorsement = "00".repeat(64); f.signed("v3/feed.json", data);
  assert.throws(() => prepareMirror(f.source, f.output, { publicKey: f.publicKey }), /Invalid publication signature/);
});

test("rejects icons that differ from the signed catalog digest", t => {
  const f = fixture(t); f.write("icons/qq.png", "changed");
  assert.throws(() => prepareMirror(f.source, f.output, { publicKey: f.publicKey }), /Invalid signed icon digest/);
});

test("rejects missing signatures and unexpected executable files", t => {
  const f = fixture(t); fs.unlinkSync(path.join(f.source, "feed.json.sig"));
  assert.throws(() => prepareMirror(f.source, f.output, { publicKey: f.publicKey }), /Missing publication file/);
  f.write("payload.js", "not publication data");
  assert.throws(() => prepareMirror(f.source, f.output, { publicKey: f.publicKey }), /Unexpected publication file/);
});

test("a partial scrape failure still mirrors its successful Pages publication", async () => {
  const repository = "null-object-0000/umanager";
  const run = { id: 42, run_attempt: 1, head_branch: "main", head_repository: { full_name: repository }, path: ".github/workflows/update-feed.yml", conclusion: "failure" };
  const result = await resolvePublication(async route => {
    if (route.startsWith("actions/workflows/")) return { workflow_runs: [run] };
    if (route.includes("/jobs?")) return { jobs: [{ name: "generate (tencent)", conclusion: "failure" }, { name: "merge", conclusion: "success", steps: [{ name: "Deploy to GitHub Pages", conclusion: "success" }] }] };
    return { artifacts: [{ name: "github-pages-42-1", expired: false }] };
  }, repository);
  assert.deepEqual(result, { id: 42, name: "github-pages-42-1" });
});

test("dispatch can resolve a completed merge while its workflow is still running", async () => {
  const repository = "null-object-0000/umanager";
  const run = { id: 43, run_attempt: 1, status: "in_progress", head_branch: "main", head_repository: { full_name: repository }, path: ".github/workflows/update-feed.yml" };
  const result = await resolvePublication(async route => {
    if (route.startsWith("actions/workflows/")) {
      assert.ok(!route.includes("status=completed"), "the workflow can still be finishing its dispatch job");
      return { workflow_runs: [run] };
    }
    if (route.includes("/jobs?")) return { jobs: [{ name: "merge", conclusion: "success", steps: [{ name: "Deploy to GitHub Pages", conclusion: "success" }] }, { name: "mirror", conclusion: null }] };
    return { artifacts: [{ name: "github-pages-43-1", expired: false }] };
  }, repository);
  assert.deepEqual(result, { id: 43, name: "github-pages-43-1" });
});

test("an unfinished Pages publication is skipped even when its artifact exists", async () => {
  const repository = "null-object-0000/umanager";
  const run = { id: 44, run_attempt: 1, head_branch: "main", head_repository: { full_name: repository }, path: ".github/workflows/update-feed.yml" };
  await assert.rejects(resolvePublication(async route => {
    if (route.startsWith("actions/workflows/")) return { workflow_runs: [run] };
    if (route.includes("/jobs?")) return { jobs: [{ name: "merge", conclusion: null, steps: [{ name: "Deploy to GitHub Pages", conclusion: null }] }] };
    throw new Error("An unpublished artifact must not be downloaded");
  }, repository), /No successful main-branch Pages publication/);
});

test("an expired latest publication fails rather than rolling back", async () => {
  const repository = "null-object-0000/umanager";
  await assert.rejects(resolvePublication(async route => {
    if (route.startsWith("actions/workflows/")) return { workflow_runs: [{ id: 42, run_attempt: 1, head_branch: "main", head_repository: { full_name: repository }, path: ".github/workflows/update-feed.yml" }] };
    if (route.includes("/jobs?")) return { jobs: [{ name: "merge", conclusion: "success", steps: [{ name: "Deploy to GitHub Pages", conclusion: "success" }] }] };
    return { artifacts: [{ name: "github-pages-42-1", expired: true }] };
  }, repository), /Latest published Pages artifact is unavailable/);
});
