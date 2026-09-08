import { defineConfig, defaultExclude } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/node_modules/**", "**/src-tauri/target/**", "**/.cargo-home/**", "**/.git/**"],
    },
  },
  test: {
    // Never scan the local toolchain dir: a Wine prefix under it must not be
    // followed (its dosdevices/z: symlinks back to /) when collecting tests.
    exclude: [...defaultExclude, "**/.tools/**"],
  },
});
