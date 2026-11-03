import { describe, expect, it } from "vitest";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, existsSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

function fixture(names, callback, symlink = false) {
  const dir = mkdtempSync(join(tmpdir(), "umanager-zip-ci-"));
  try {
    const zip = join(dir, "vendor.zip"), out = join(dir, "package.deb");
    execFileSync("python3", ["-c", `
import json, sys, zipfile
with zipfile.ZipFile(sys.argv[1], 'w', zipfile.ZIP_DEFLATED) as z:
 for name in json.loads(sys.argv[2]):
  info=zipfile.ZipInfo(name)
  if sys.argv[3]=='true': info.external_attr=0o120777 << 16
  z.writestr(info,b'test package')
`, zip, JSON.stringify(names), String(symlink)]);
    callback(zip, out);
  } finally { rmSync(dir, { recursive: true, force: true }); }
}

describe("CI ZIP DEB extraction", () => {
  it("extracts only the one regular package and reports its exact member name", () => {
    fixture(["apifox_1_amd64.deb", "readme.txt"], (zip, out) => {
      const result = execFileSync("python3", ["scripts/extract-zip-deb.py", zip, out]);
      expect(JSON.parse(result)).toEqual({ member: "apifox_1_amd64.deb" });
      expect(readFileSync(out, "utf8")).toBe("test package");
    });
  });
  it("rejects traversal, ambiguous packages and symlinks without creating output", () => {
    for (const [names, symlink] of [[["../app.deb"], false], [["app.deb", "other.deb"], false], [["app.deb"], true]]) {
      fixture(names, (zip, out) => {
        expect(() => execFileSync("python3", ["scripts/extract-zip-deb.py", zip, out], { stdio: "pipe" })).toThrow();
        expect(existsSync(out)).toBe(false);
      }, symlink);
    }
  });
  it("never replaces or deletes an existing output", () => {
    fixture(["app.deb"], (zip, out) => {
      writeFileSync(out, "keep");
      expect(() => execFileSync("python3", ["scripts/extract-zip-deb.py", zip, out], { stdio: "pipe" })).toThrow();
      expect(readFileSync(out, "utf8")).toBe("keep");
    });
  });
});
