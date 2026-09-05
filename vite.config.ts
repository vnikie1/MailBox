import { defineConfig, type Plugin } from 'vite'
import react from '@vitejs/plugin-react'
import { fileURLToPath, URL } from 'node:url'
import { readdir, readFile } from 'node:fs/promises'
import { join, resolve } from 'node:path'

// Tauri expects a fixed port and must not have the dev server clear the terminal,
// because the Rust build output shares it.
const host = process.env.TAURI_DEV_HOST

/**
 * pdf.js's font and CMap data, served in dev and shipped in the build.
 *
 * The attachment previewer renders PDFs itself (see `PdfView.tsx` for why it cannot hand them
 * to the WebView), and pdf.js reaches for two data directories at runtime:
 *
 * - `standard_fonts` — the fourteen fonts a PDF is allowed to *reference* without embedding.
 *   Generated documents do this constantly, so without these an invoice renders as blank
 *   boxes. 820KB.
 * - `cmaps` — character maps for CJK and other non-Latin encodings. 1.5MB.
 *
 * They are copied rather than imported because pdf.js builds these URLs at runtime from a
 * directory prefix, so Vite never sees an import to follow and hashed filenames would not be
 * findable. `emitFile` with an explicit `fileName` is the escape hatch for exactly that.
 *
 * Kept out of `public/` deliberately: these are 2.3MB of vendored binaries that would
 * otherwise be committed and then drift from whatever version of pdf.js is installed.
 */
const PDFJS_ROOT = fileURLToPath(new URL('./node_modules/pdfjs-dist', import.meta.url))
const PDFJS_DATA_DIRS = ['standard_fonts', 'cmaps']

function pdfjsData(): Plugin {
  return {
    name: 'halcyon-pdfjs-data',

    configureServer(server) {
      server.middlewares.use('/pdfjs/', (request, response, next) => {
        const path = decodeURIComponent((request.url ?? '').split('?')[0] ?? '')
        const segments = path.split('/').filter(Boolean)
        const [directory] = segments

        // Allowlisted by first segment rather than by resolving and comparing prefixes: the
        // set of directories this serves is fixed and tiny, so there is nothing to traverse to.
        if (
          segments.length !== 2 ||
          directory === undefined ||
          !PDFJS_DATA_DIRS.includes(directory)
        ) {
          next()
          return
        }

        readFile(resolve(PDFJS_ROOT, ...segments))
          .then((body) => {
            response.setHeader('Content-Type', 'application/octet-stream')
            response.end(body)
          })
          .catch(() => {
            next()
          })
      })
    },

    async generateBundle() {
      for (const directory of PDFJS_DATA_DIRS) {
        const names = await readdir(join(PDFJS_ROOT, directory))

        for (const name of names) {
          this.emitFile({
            type: 'asset',
            fileName: `pdfjs/${directory}/${name}`,
            source: await readFile(join(PDFJS_ROOT, directory, name)),
          })
        }
      }
    },
  }
}

export default defineConfig({
  plugins: [react(), pdfjsData()],
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
