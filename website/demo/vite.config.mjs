import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";
import path from "node:path";

const directory = path.dirname(fileURLToPath(import.meta.url));
const bridge = path.join(directory, "native-bridge.ts");
const modules = ["core", "event", "app", "webviewWindow"];

export default defineConfig({
  root: directory,
  base: "/demo/",
  resolve: { alias: modules.map(name => ({ find: `@tauri-apps/api/${name}`, replacement: bridge })) },
  plugins: [
    {
      name: "isolated-website-demo",
      enforce: "pre",
      resolveId(source) {
        if (source.startsWith("@tauri-apps/")) throw new Error(`Unmapped native dependency in website demo: ${source}`);
      },
      transform(code, id) {
        if (!/\/src\/(?:api|batchApi)\.ts$/.test(id.replaceAll("\\", "/"))) return;
        const guard = 'import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)';
        if (!code.includes(guard)) throw new Error("The preview guard changed; review the demo isolation before publishing");
        // Enable only the existing preview API branches, only in this build.
        return { code: code.replaceAll(guard, "true").replace('appVersion: "0.1.0"', 'appVersion: "交互演示"'), map: null };
      },
    },
    react(),
  ],
  build: { outDir: "../dist/demo", emptyOutDir: true },
});
