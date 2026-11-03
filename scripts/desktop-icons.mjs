import { readFileSync, readdirSync, realpathSync, statSync } from "node:fs";
import { isAbsolute, join, relative, resolve } from "node:path";

// Some packages keep their icon in a private application directory instead of
// hicolor/pixmaps (DBeaver: /usr/share/dbeaver-ce/dbeaver.png). Follow only the
// absolute Icon path declared by the main desktop entry, within the extracted
// package. Never execute desktop entries or follow links outside that tree.
export function desktopIconPaths(extractDir) {
  const root = realpathSync(extractDir);
  const containedFile = (path) => {
    try {
      const actual = realpathSync(path);
      const rel = relative(root, actual);
      return rel && rel !== ".." && !rel.startsWith("../") && !isAbsolute(rel)
        && statSync(actual).isFile() ? actual : null;
    } catch {
      return null;
    }
  };
  let entries;
  const applications = join(root, "usr/share/applications");
  try {
    entries = readdirSync(applications);
  } catch {
    return [];
  }
  const icons = new Set();
  for (const name of entries) {
    if (!name.endsWith(".desktop")) continue;
    const desktop = containedFile(join(applications, name));
    if (!desktop) continue;
    let mainEntry = false;
    for (const line of readFileSync(desktop, "utf8").split(/\r?\n/)) {
      const text = line.trim();
      if (text.startsWith("[")) mainEntry = text === "[Desktop Entry]";
      if (!mainEntry || !text.startsWith("Icon=")) continue;
      const icon = text.slice(5).trim();
      if (!isAbsolute(icon) || !/\.(png|svg)$/i.test(icon)) continue;
      const path = containedFile(resolve(root, `.${icon}`));
      if (path) icons.add(path);
    }
  }
  return [...icons];
}
