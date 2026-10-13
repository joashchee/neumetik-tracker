import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
// @ts-expect-error type error without @types/node package
import process from "node:process";

const host = process.env.TAURI_DEV_HOST;
const pkg = JSON.parse(readFileSync(fileURLToPath(new URL("./package.json", import.meta.url)), "utf-8"));

// The web build is served from a folder (GitHub Pages: /neumetik-tracker/);
// BASE says which. The apps load from their root.
export default defineConfig(() => ({
  base: process.env.BASE ?? "/",
  plugins: [react()],
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
  },
  clearScreen: false,
  server: {
    port: 1480,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1481 } : undefined,
    watch: { ignored: ["**/src-tauri/**", "**/target/**"] },
  },
}));
