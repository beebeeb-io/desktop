import { defineConfig } from "vite";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath } from "node:url";

const host = process.env.TAURI_DEV_HOST;
const shared = (p: string) => fileURLToPath(new URL(`./packages/shared/src/${p}`, import.meta.url));

export default defineConfig({
  clearScreen: false,
  plugins: [tailwindcss()],
  resolve: {
    // The mirrored @beebeeb/shared subpaths desktop may use (packages/shared/MIRROR.md). There is
    // deliberately no alias for "@beebeeb/shared/api": the web view never holds a session token.
    alias: {
      "@beebeeb/shared/design": shared("design/index.ts"),
      "@beebeeb/shared/drive-ui": shared("drive-ui/index.ts"),
      "@beebeeb/shared/tokens.css": shared("tokens.css"),
    },
  },
  server: {
    port: 5173,
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: "ws", host, port: 5174 }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
    // The same isolation the Tauri main window carries (src-tauri/tauri.conf.json app.security.headers); the test
    // reads both sides. A plain object, not a function, so a static read of this file sees it.
    headers: {
      "Cross-Origin-Opener-Policy": "same-origin",
      "Cross-Origin-Embedder-Policy": "require-corp",
    },
  },
});
