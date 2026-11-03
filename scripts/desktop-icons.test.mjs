import { afterEach, describe, expect, it } from "vitest";
import { mkdtempSync, mkdirSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { desktopIconPaths } from "./desktop-icons.mjs";

const roots = [];
afterEach(() => roots.splice(0).forEach((root) => rmSync(root, { recursive: true, force: true })));
function fixture() {
  const root = mkdtempSync(join(tmpdir(), "umanager-desktop-icons-"));
  roots.push(root);
  const write = (path, text = "icon") => {
    const full = join(root, path);
    mkdirSync(join(full, ".."), { recursive: true });
    writeFileSync(full, text);
    return full;
  };
  return { root, write };
}

describe("desktopIconPaths", () => {
  it("finds a DBeaver-style private icon, ignoring action icons and Exec", () => {
    const { root, write } = fixture();
    const icon = write("usr/share/dbeaver-ce/dbeaver.png");
    write("usr/share/applications/dbeaver.desktop", "[Desktop Entry]\nIcon=/usr/share/dbeaver-ce/dbeaver.png\nExec=touch /tmp/never-run\n[Desktop Action Other]\nIcon=/other.png\n");
    write("other.png");
    expect(desktopIconPaths(root)).toEqual([icon]);
  });

  it("rejects traversal, escaped symlinks and desktop files outside the package", () => {
    const { root, write } = fixture();
    const outside = fixture();
    const externalIcon = outside.write("external.png");
    write("usr/share/applications/test.desktop", `[Desktop Entry]\nIcon=/../../${externalIcon}\nIcon=/linked.png\nIcon=theme-icon\nIcon=/missing.svg\n`);
    symlinkSync(externalIcon, join(root, "linked.png"));
    const externalDesktop = outside.write("external.desktop", "[Desktop Entry]\nIcon=/valid.png\n");
    write("valid.png");
    symlinkSync(externalDesktop, join(root, "usr/share/applications/linked.desktop"));
    expect(desktopIconPaths(root)).toEqual([]);
  });

  it("allows internal symlinks and deduplicates declared icons", () => {
    const { root, write } = fixture();
    const icon = write("private/icon.svg");
    symlinkSync("private/icon.svg", join(root, "icon.svg"));
    for (const name of ["one", "two"]) write(`usr/share/applications/${name}.desktop`, "[Desktop Entry]\r\nIcon=/icon.svg\r\n");
    expect(desktopIconPaths(root)).toEqual([icon]);
  });

  it("accepts packages without desktop files", () => {
    expect(desktopIconPaths(fixture().root)).toEqual([]);
  });
});
