import { defineConfig, devices } from '@playwright/test'

/**
 * Phase 0 e2e runs the UI shell in Chromium against the Vite dev server. That covers
 * everything the WebView renders — tokens, theme swapping, caption-button visuals,
 * focus order — and is what the Phase 1 gallery visual baseline will use.
 *
 * It deliberately does NOT cover the Win32 half (Snap Layouts, DWM backdrop, real OS
 * theme changes), which cannot be driven from a browser. Those are verified against the
 * running Tauri window; see docs/PHASE-0-VERIFICATION.md.
 */
export default defineConfig({
  testDir: './tests/e2e',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  // Serialised on CI so the perf-sensitive assertions are not fighting for cores.
  ...(process.env.CI ? { workers: 1 } : {}),
  reporter: process.env.CI ? [['github'], ['html', { open: 'never' }]] : 'list',
  use: {
    baseURL: 'http://localhost:1420',
    trace: 'on-first-retry',
  },
  expect: {
    toHaveScreenshot: { maxDiffPixelRatio: 0.002 },
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'], viewport: { width: 1400, height: 900 } },
    },

    /**
     * Windows display scaling. docs/02 §8 requires 100 / 125 / 150 / 175 % and the
     * definition of done repeats it — this is a Windows app and 125 % is the factory
     * setting on most laptops it will run on, so 100 % alone is the unusual case.
     *
     * Only the scaling spec runs here: re-running the whole suite four times would
     * quadruple CI for no signal, since nothing else is scale-dependent.
     */
    ...[1.25, 1.5, 1.75].map((scale) => ({
      name: `chromium@${String(scale)}x`,
      testMatch: /scaling.spec.ts/,
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 1400, height: 900 },
        deviceScaleFactor: scale,
      },
    })),
  ],
  webServer: {
    command: 'npm run dev',
    url: 'http://localhost:1420',
    reuseExistingServer: !process.env.CI,
    stdout: 'ignore',
    stderr: 'pipe',
  },
})
