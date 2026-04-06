import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;
// @ts-expect-error process is a nodejs global
const isWindows = process.env.TAURI_ENV_PLATFORM === 'windows';
// @ts-expect-error process is a nodejs global
const isDebug = !!process.env.TAURI_ENV_DEBUG;

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [sveltekit()],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
  // Env variables starting with the item of `envPrefix` will be exposed in tauri's source code through `import.meta.env`.
  envPrefix: ['VITE_', 'TAURI_ENV_*'],
  build: {
    // Tauri defaults to Chromium/WebKit 97+.
    target: isWindows ? 'chrome105' : 'safari13',
    // don't minify for debug builds
    minify: !isDebug ? /** @type {const} */ ('esbuild') : false,
    // produce sourcemaps for debug builds
    sourcemap: isDebug,
    rollupOptions: {
      output: {
        manualChunks: undefined // Let Vite/Rollup chunk efficiently
      }
    }
  },
}));
