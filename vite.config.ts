import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { fileURLToPath, URL } from 'node:url'

// Tauri expects a fixed port and must not have the dev server clear the terminal,
// because the Rust build output shares it.
const host = process.env.TAURI_DEV_HOST

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host ?? false,
    // Spread rather than assign undefined: exactOptionalPropertyTypes is on, and an
    // explicit undefined is not the same as an absent key.
    ...(host ? { hmr: { protocol: 'ws' as const, host, port: 1421 } } : {}),
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
  // Tauri ships a fixed WebView2 (Chromium) on Windows, so we can target it directly
  // instead of shipping legacy transpilation nobody will ever run.
  build: {
    target: 'chrome110',
    minify: process.env.TAURI_ENV_DEBUG ? false : 'esbuild',
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
  envPrefix: ['VITE_', 'TAURI_ENV_'],
})
