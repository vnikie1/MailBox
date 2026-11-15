import { expect, test, type Locator, type Page } from '@playwright/test'

/**
 * Dragging a recipient chip from one field to another, with a real pointer.
 *
 * M48 in the 2026-11-14 audit. This was HTML5 drag and drop, and in the compose window Tauri's
 * native drop handler — kept on so that files dragged in from Explorer arrive with their paths —
 * takes every HTML5 drag in the page, so no chip could be moved. It is pointer events now, which
 * that handler leaves alone. The compose window does not start in a browser (it needs the
 * core), so this drives the gallery's two fields that share a drag group.
 */

function field(page: Page, name: string): Locator {
  // Exact, or "Cc:" also finds the "Bcc:" specimen.
  return page.getByRole('region', { name: 'Light' }).getByRole('group', { name, exact: true })
}

/** The chips in a field, by the name each shows. Read from the remove buttons, which name them. */
function chips(page: Page, name: string): Promise<string[]> {
  return field(page, name)
    .getByRole('button', { name: /^Remove / })
    .evaluateAll((buttons) =>
      buttons.map((button) => (button.getAttribute('aria-label') ?? '').replace(/^Remove /, '')),
    )
}

/** Presses on `from`, moves to the middle of `to` in steps, and lets go. */
async function drag(page: Page, from: Locator, to: Locator) {
  // On screen first: the field under the pointer is found by hit-testing, and nothing off
  // screen can be hit.
  await to.scrollIntoViewIfNeeded()
  await from.scrollIntoViewIfNeeded()

  const start = await from.boundingBox()
  const end = await to.boundingBox()
  if (start === null || end === null) throw new Error('nothing to drag between')

  await page.mouse.move(start.x + start.width / 2, start.y + start.height / 2)
  await page.mouse.down()
  await page.mouse.move(end.x + end.width / 2, end.y + end.height / 2, { steps: 8 })
  await page.mouse.up()
}

test.beforeEach(async ({ page }) => {
  await page.goto('/dev/gallery')
  await expect(field(page, 'To:')).toBeVisible()
})

test('a chip dragged to another field moves there', async ({ page }) => {
  await drag(
    page,
    field(page, 'To:').getByText('not an address'),
    field(page, 'Cc:').getByRole('textbox'),
  )

  await expect.poll(() => chips(page, 'Cc:')).toContain('not an address')
  await expect.poll(() => chips(page, 'To:')).not.toContain('not an address')
})

test('a chip dragged back onto its own field stays', async ({ page }) => {
  await drag(
    page,
    field(page, 'To:').getByText('Ada Lovelace'),
    field(page, 'To:').getByRole('textbox'),
  )

  expect(await chips(page, 'To:')).toEqual(['Ada Lovelace', 'grace@example.com', 'not an address'])
})

test('a click on a chip’s remove button is still a click', async ({ page }) => {
  await field(page, 'To:').getByRole('button', { name: 'Remove Ada Lovelace' }).click()

  expect(await chips(page, 'To:')).toEqual(['grace@example.com', 'not an address'])
})
