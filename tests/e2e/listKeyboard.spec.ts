import { expect, test } from '@playwright/test'

/**
 * Walking the message list with the arrow keys after clicking a row.
 *
 * M40 in the 2026-11-14 audit. A row took focus when it was clicked, and the list is virtualised:
 * arrowing down scrolled that row out of the rendered window, it was unmounted, focus fell to the
 * page, and the keys reached nothing. Holding ArrowDown stopped after about twenty rows. Only a
 * real browser runs the virtualiser and moves focus, so this is an end-to-end test.
 */
test('the arrow keys keep moving the selection far past the row that was clicked', async ({
  page,
}) => {
  await page.goto('/')
  const list = page.getByRole('listbox', { name: 'Messages' })
  await expect(list).toBeVisible()

  const rows = page.getByRole('option')
  await rows.first().click()
  const first = await page.locator('[role="option"][aria-selected="true"]').getAttribute('id')

  // Far enough that the clicked row has long left the rendered window.
  const steps = 40
  for (let step = 0; step < steps; step += 1) {
    await page.keyboard.press('ArrowDown')
  }

  // Focus is still on the list, which says which row is current.
  await expect(list).toBeFocused()
  const current = await list.getAttribute('aria-activedescendant')
  expect(current).not.toBe(first)

  await expect(page.locator(`[id="${current ?? ''}"]`)).toHaveAttribute('aria-selected', 'true')

  // Every press counted: as many back up lands exactly on the row that was clicked. A walk that
  // had stalled part of the way down would overshoot it and stop at the top.
  for (let step = 0; step < steps; step += 1) {
    await page.keyboard.press('ArrowUp')
  }
  await expect(list).toHaveAttribute('aria-activedescendant', first ?? '')
})
