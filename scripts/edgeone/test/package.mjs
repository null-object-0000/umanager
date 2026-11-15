import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { generateKeyPairSync, sign, createHash } from "node:crypto";
import { stagePackage } from "../package.mjs";
function fixture(t) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "umanager-package-test-"));
  t.after(() => {
    if (!path.resolve(directory).startsWith(path.resolve(os.tmpdir()) + path.sep + "umanager-package-test-")) throw new Error("Unsafe test cleanup");
    fs.rmSync(directory, { recursive: true, force: true });
  });
  const key = generateKeyPairSync("ed25519"), publicKey = key.publicKey.export({ format: "der", type: "spki" }).subarray(-32).toString("hex");
  const packageBytes = Buffer.from("original release package bytes");
  const update = { version: "0.18.5", packageName: "u-manager", architecture: "amd64", size: packageBytes.length, sha256: createHash("sha256").update(packageBytes).digest("hex"), assetName: "UManager_0.18.5_amd64.deb", releaseTag: "v0.18.5", downloadUrl: "https://github.com/null-object-0000/umanager/releases/download/v0.18.5/UManager_0.18.5_amd64.deb" };
  function write() {
    const feed = path.join(directory, "umanager/v3/feed.json"), bytes = Buffer.from(JSON.stringify({ schemaVersion: 3, selfUpdate: update }));
    fs.mkdirSync(path.dirname(feed), { recursive: true }); fs.writeFileSync(feed, bytes); fs.writeFileSync(feed + ".sig", sign(null, bytes, key.privateKey).toString("hex"));
  }
  write(); return { directory, publicKey, packageBytes, update, write };
}
test("package publication copies exactly the signed release bytes through approved redirects", async t => {
  const f = fixture(t); let calls = 0;
  const result = await stagePackage(f.directory, { publicKey: f.publicKey, fetchImpl: async (url, options) => {
    assert.equal(options.redirect, "manual"); calls++;
    return calls === 1 ? new Response(null, { status: 302, headers: { location: "https://release-assets.githubusercontent.com/asset" } }) : new Response(f.packageBytes);
  } });
  assert.equal(calls, 2); assert.deepEqual(fs.readFileSync(path.join(f.directory, result.path)), f.packageBytes);
});
test("invalid feed signatures never initiate a package request", async t => {
  const f = fixture(t); fs.appendFileSync(path.join(f.directory, "umanager/v3/feed.json"), " ");
  await assert.rejects(stagePackage(f.directory, { publicKey: f.publicKey, fetchImpl: () => { throw new Error("must not fetch"); } }), /Invalid publication signature/);
});
test("invalid signed release identity, path traversal and free-tier overflow are rejected", async t => {
  for (const changes of [{ packageName: "other" }, { version: "../../escape" }, { size: 26 * 1024 * 1024 }, { downloadUrl: "https://github.com/other/release.deb" }, { assetName: "other.deb" }]) {
    const f = fixture(t); Object.assign(f.update, changes); f.write();
    await assert.rejects(stagePackage(f.directory, { publicKey: f.publicKey, fetchImpl: () => { throw new Error("must not fetch"); } }), /Unsupported signed|Unexpected canonical/);
  }
});
test("redirects cannot escape HTTPS and the exact GitHub asset host allowlist", async t => {
  for (const location of ["https://github.com.evil.test/file", "http://github.com/file", "https://updates.snewbie.site/file", "https://github.com:8443/file", "https://user:pass@github.com/file"]) {
    const f = fixture(t); let calls = 0;
    await assert.rejects(stagePackage(f.directory, { publicKey: f.publicKey, fetchImpl: async () => { calls++; return new Response(null, { status: 302, headers: { location } }); } }), /Untrusted package redirect/);
    assert.equal(calls, 1);
  }
});
test("missing, truncated, oversized and same-size corrupted packages are never staged", async t => {
  for (const response of [() => new Response(null, { status: 404 }), () => new Response("x"), () => new Response(Buffer.alloc(100)), () => new Response(Buffer.alloc(30)), () => new Response("x", { headers: { "content-length": "1" } })]) {
    const f = fixture(t);
    await assert.rejects(stagePackage(f.directory, { publicKey: f.publicKey, fetchImpl: async () => response() }));
    assert.equal(fs.existsSync(path.join(f.directory, "umanager/releases")), false);
    assert.equal(fs.existsSync(path.join(f.directory, "package-status.json")), false);
  }
});
