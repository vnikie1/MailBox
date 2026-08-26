import { expect, test } from '@playwright/test'

/**
 * Display scaling and contrast — the two `docs/02` §8 checklist items that were still
 * unverified at the end of Phase 2.
 *
 * Scaling runs under four Playwright projects (100 / 125 / 150 / 175 %), configured in
 * playwright.config.ts. What is being checked is not that the pixels differ — they will —
 * but that the *layout* is identical in CSS pixels at every scale. A layout that drifts
 * with the scale factor means something is measured in device pixels somewhere, and the
 * usual culprit is a JavaScript measurement like the ones in lib/tokens.ts.
 */

const LAYOUT_KEY = 'halcyon.settings.layout'
const SETTINGS_KEY = 'halcyon.settings.display'

test.beforeEach(async ({ page }) => {
  await page.addInitScript(
    ([settingsKey, layoutKey]) => {
      window.localStorage.setItem(
        settingsKey,
        JSON.stringify({
          state: { theme: 'light', density: 'default', transparency: 'system' },
          version: 0,
        }),
      )
      window.localStorage.setItem(
        layoutKey,
        JSON.stringify({
          state: {
            sidebarWidth: 232,
            listWidth: 360,
            sidebarCollapsed: false,
            classicLayout: false,
            previewLines: 2,
            collapsedSections: ['all-drafts', 'all-sent'],
            sortField: 'date',
            sortAscending: false,
            organiseByConversation: true,
          },
          version: 0,
        }),
      )
    },
    [SETTINGS_KEY, LAYOUT_KEY] as const,
  )
})

test('lays out identically at every Windows display scale', async ({ page }) => {
  await page.goto('/')
  await expect(page.getByRole('listbox', { name: 'Messages' })).toBeVisible()
  await page.evaluate(() => document.fonts.ready)

  const metrics = await page.evaluate(() => {
    const rect = (selector: string) => {
      const element = document.querySelector(selector)
      return element ? element.getBoundingClientRect() : null
    }

    return {
      dpr: window.devicePixelRatio,
      toolbar: rect('header')?.height ?? 0,
      sidebar: rect('[role="tree"]')?.width ?? 0,
      list: rect('[role="listbox"]')?.width ?? 0,
      row: rect('[role="option"]')?.height ?? 0,
      treeitem: rect('[role="treeitem"]')?.height ?? 0,
    }
  })

  // Identical in CSS pixels whatever the device pixel ratio. If any of these drift, a
  // measurement somewhere is in device pixels rather than CSS pixels.
  expect(metrics.toolbar).toBe(52)
  expect(metrics.sidebar).toBe(232)
  expect(metrics.list).toBe(360)
  expect(metrics.row).toBe(80)
  expect(metrics.treeitem).toBe(32)
})

test('renders text crisply rather than upscaling a bitmap', async ({ page }) => {
  await page.goto('/')
  await expect(page.getByRole('listbox', { name: 'Messages' })).toBeVisible()

  // A CSS transform-based zoom would leave devicePixelRatio at 1 and blur everything. The
  // scale has to reach the compositor for text to be re-rasterised at the higher density.
  const dpr = await page.evaluate(() => window.devicePixelRatio)
  const expected = /@([\d.]+)x/.exec(test.info().project.name)?.[1]
  if (expected) expect(dpr).toBeCloseTo(Number(expected), 2)
})

/**
 * docs/02 §8 — "`--label-2` on `--bg-content` >= 4.5:1 for anything the user must read".
 *
 * Measured rather than eyeballed. --label-2 is an alpha colour, so it has to be composited
 * over the background before the ratio means anything; comparing the raw rgba against the
 * background is the mistake this test exists to prevent.
 */
test('meets the contrast floor for secondary text in both themes', async ({ page }) => {
  await page.goto('/')
  await expect(page.getByRole('listbox', { name: 'Messages' })).toBeVisible()

  for (const theme of ['light', 'dark'] as const) {
    const ratio = await page.evaluate((mode) => {
      document.documentElement.dataset.theme = mode

      const parse = (value: string): [number, number, number, number] => {
        const parts = value.match(/[\d.]+/g)?.map(Number) ?? []
        return [parts[0] ?? 0, parts[1] ?? 0, parts[2] ?? 0, parts[3] ?? 1]
      }

      const style = getComputedStyle(document.documentElement)
      const probe = document.createElement('span')
      probe.style.color = style.getPropertyValue('--label-2')
      probe.style.backgroundColor = style.getPropertyValue('--bg-content')
      document.body.append(probe)

      const probeStyle = getComputedStyle(probe)
      const [fr, fg, fb, fa] = parse(probeStyle.color)
      const [br, bg, bb] = parse(probeStyle.backgroundColor)
      probe.remove()

      // Composite the translucent label over the opaque ground first.
      const over = (f: number, b: number) => f * fa + b * (1 - fa)
      const composited: [number, number, number] = [over(fr, br), over(fg, bg), over(fb, bb)]

      const luminance = (rgb: [number, number, number]) => {
        const [r, g, b] = rgb.map((channel) => {
          const c = channel / 255
          return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4)
        }) as [number, number, number]
        return 0.2126 * r + 0.7152 * g + 0.0722 * b
      }

      const l1 = luminance(composited)
      const l2 = luminance([br, bg, bb])
      return (Math.max(l1, l2) + 0.05) / (Math.min(l1, l2) + 0.05)
    }, theme)

    console.log(`--label-2 on --bg-content, ${theme}: ${ratio.toFixed(2)}:1`)
    expect(ratio, `${theme} secondary text contrast`).toBeGreaterThanOrEqual(4.5)
  }
})
