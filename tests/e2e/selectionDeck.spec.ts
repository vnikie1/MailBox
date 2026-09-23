import { expect, test, type Locator, type Page } from '@playwright/test'

/**
 * The reader's selection stack — what shows when several rows are selected at once.
 *
 * Driven in a browser because the whole thing is layout: sheets the size of the pane in one grid
 * cell, each tilted about its middle and lifted by a stagger that is a `calc()` over the type
 * scale. None of that has a return value a unit test could assert. The failure mode is a sheet's
 * header sliding under the sheet in front of it, which is a geometry question, and only a real
 * layout engine answers those.
 *
 * `tests/unit/selectionDeck.test.tsx` covers the decisions: which messages get a sheet, which
 * sheet has a body, and that the body is asked for with remote images off.
 */

/** The caption, which is also the live region the count is announced from. */
function caption(page: Page): Locator {
  return page.getByRole('status').filter({ hasText: 'Messages Selected' })
}

/** The sheets, deepest first — the order they are appended in. */
function sheets(page: Page): Locator {
  return page.locator('[data-sheets] > article')
}

/** Selects the first `count` rows as a run, the way a shift-click does. */
async function selectRows(page: Page, count: number): Promise<void> {
  const rows = page.getByRole('option')
  await rows.first().click()
  await rows.nth(count - 1).click({ modifiers: ['Shift'] })
  await expect(page.locator('[role="option"][aria-selected="true"]')).toHaveCount(count)
}

interface Corners {
  top: number
  bottom: number
  left: number
  right: number
  headerBottom: number
}

/** Every sheet's box and the bottom of its header, as the renderer drew them. */
async function measure(page: Page): Promise<Corners[]> {
  return sheets(page).evaluateAll((elements) =>
    elements.map((element) => {
      const box = element.getBoundingClientRect()
      const header = element.querySelector('header')?.getBoundingClientRect()
      return {
        top: box.top,
        bottom: box.bottom,
        left: box.left,
        right: box.right,
        headerBottom: header?.bottom ?? Number.NaN,
      }
    }),
  )
}

test.describe('the selection stack', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await expect(page.getByRole('listbox', { name: 'Messages' })).toBeVisible()
  })

  test('one message still opens the message', async ({ page }) => {
    await page.getByRole('option').first().click()

    await expect(caption(page)).toHaveCount(0)
    await expect(page.getByText(/^\d+ Messages?$/)).toBeVisible()
  })

  test('two messages draw two sheets and the count', async ({ page }) => {
    await selectRows(page, 2)

    await expect(sheets(page)).toHaveCount(2)
    await expect(caption(page)).toContainText('2 Messages Selected')
  })

  test('the stack stops at three sheets however many are selected', async ({ page }) => {
    await selectRows(page, 7)

    await expect(sheets(page)).toHaveCount(3)
    // The count is the whole selection, not what is drawn. A stack that said "3" while seven
    // rows were lit would be the app understating what Delete is about to take.
    await expect(caption(page)).toContainText('7 Messages Selected')
  })

  test('the sheet on top is the first message selected, with its body', async ({ page }) => {
    await selectRows(page, 3)
    await expect(sheets(page)).toHaveCount(3)

    const sender = (await page.getByRole('option').first().innerText()).split('\n')[0] ?? ''
    expect(sender).not.toBe('')

    // Appended deepest first, so the last in the DOM is the one on top.
    const front = sheets(page).last()
    await expect(front).toContainText(sender)
    await expect(front.locator('iframe')).toHaveCount(1)

    // And the two behind have none: a header, and blank paper under it.
    await expect(sheets(page).first().locator('iframe')).toHaveCount(0)
    await expect(sheets(page).nth(1).locator('iframe')).toHaveCount(0)
  })

  test('every header behind is clear of the sheet in front of it', async ({ page }) => {
    await selectRows(page, 3)
    await expect(sheets(page)).toHaveCount(3)

    // The geometry the stylesheet argues for, measured rather than trusted. A sheet behind is
    // visible only down to the top edge of the sheet in front, so its header must finish above
    // that edge. Measured the strict way: a rotated box's bottom is its lowest corner and its top
    // its highest, and those are at opposite ends — so passing this, the real gap is wider.
    const boxes = await measure(page)
    expect(boxes).toHaveLength(3)

    for (let index = 0; index < boxes.length - 1; index += 1) {
      const behind = boxes[index]
      const inFront = boxes[index + 1]
      if (behind === undefined || inFront === undefined) throw new Error('missing sheet')

      expect(Number.isNaN(behind.headerBottom)).toBe(false)
      expect(behind.headerBottom).toBeLessThanOrEqual(inFront.top)
    }
  })

  test('the stack fills the pane and stays inside it', async ({ page }) => {
    await selectRows(page, 3)
    await expect(sheets(page)).toHaveCount(3)

    const pane = await page.locator('[data-sheets]').evaluate((stage) => {
      const box = (stage.parentElement ?? stage).getBoundingClientRect()
      return { top: box.top, bottom: box.bottom, left: box.left, right: box.right }
    })
    const boxes = await measure(page)
    const front = boxes[boxes.length - 1]
    if (front === undefined) throw new Error('no front sheet')

    // "Big and fill the whole mail window": the sheet on top takes most of the pane's height,
    // not a card's worth in the middle of it.
    expect(front.bottom - front.top).toBeGreaterThan((pane.bottom - pane.top) * 0.5)

    // Every corner of every sheet, tilted or not, lands inside the pane. The lifted ones are out
    // of the flow, so this is what says the stage reserved their room; the tilted ones swing
    // sideways, so this is what says the margin is wide enough for the swing.
    for (const box of boxes) {
      expect(box.top).toBeGreaterThanOrEqual(pane.top)
      expect(box.left).toBeGreaterThanOrEqual(pane.left)
      expect(box.right).toBeLessThanOrEqual(pane.right)
      expect(box.bottom).toBeLessThanOrEqual(pane.bottom)
    }
  })
})
