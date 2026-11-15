import { expect, test, type Locator } from '@playwright/test'

/**
 * The search field and the scope bar fit the window they are in.
 *
 * M43 in the 2026-11-14 audit: at the default window size the scope bar was clipped ("200 re…")
 * and the search field, which widens when focused, ran off the right edge of the window. Both are
 * geometry, so this measures what the browser drew.
 */

/** The size `tauri.conf.json` opens the window at. */
const DEFAULT_WINDOW = { width: 1400, height: 900 }

/** What, inside the element, is drawn past its edges. Empty when everything fits. */
async function overflowing(element: Locator): Promise<string[]> {
  return element.evaluate((box) => {
    const edges = box.getBoundingClientRect()
    const out: string[] = []

    if (box.scrollWidth > box.clientWidth + 1) {
      out.push(`scrolls ${String(box.scrollWidth - box.clientWidth)}px`)
    }
    for (const child of box.querySelectorAll('*')) {
      const inner = child.getBoundingClientRect()
      if (inner.width > 0 && (inner.right > edges.right + 1 || inner.left < edges.left - 1)) {
        out.push(`"${child.textContent.trim().slice(0, 30)}" spills past the edge`)
      }
    }
    return out
  })
}

test.describe('at the default window size', () => {
  test.use({ viewport: DEFAULT_WINDOW })

  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await expect(page.getByRole('listbox', { name: 'Messages' })).toBeVisible()

    const field = page.locator('input[data-shortcut="search"]')
    await field.click()
    await field.fill('quarterly')
  })

  test('the focused search field stays inside the window', async ({ page }) => {
    const field = page.locator('input[data-shortcut="search"]')
    const box = await field.boundingBox()
    expect(box).not.toBeNull()
    expect((box?.x ?? 0) + (box?.width ?? 0)).toBeLessThanOrEqual(DEFAULT_WINDOW.width)

    // The toolbar the field sits at the end of: the nearest ancestor holding the Archive button.
    const toolbar = page
      .locator('div')
      .filter({ has: page.getByRole('button', { name: 'Archive' }) })
      .filter({ has: field })
      .last()
    expect(await overflowing(toolbar)).toEqual([])
  })

  test('the scope bar shows all of itself', async ({ page }) => {
    const bar = page.getByRole('radiogroup', { name: 'Where to search' }).locator('..')
    await expect(page.getByRole('button', { name: 'Save Search…' })).toBeVisible()
    expect(await overflowing(bar)).toEqual([])
  })
})
