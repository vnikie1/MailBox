import { expect, test } from '@playwright/test'

/**
 * Show Unread Only must not read the mailbox by itself.
 *
 * H1 in the 2026-11-14 audit, reproduced there with this exact sequence: one click with the
 * filter on marked 11 messages read in 8 seconds and opened 8 of them. Opening a message marks
 * it read after 700ms; the refetch dropped it from the unread-only list; the selection followed
 * the list to the row that took its place; that row was marked read 700ms later; and so on down
 * the mailbox, each one synced to the server as a read.
 *
 * Mail keeps the open message in a filtered list until the selection moves. So does Halcyon now.
 */

const selected = '[role=option][aria-selected=true]'

test('opening one message with Show Unread Only on reads that message and no other', async ({
  page,
}) => {
  await page.goto('/')
  await expect(page.locator('[role=option]').first()).toBeVisible()

  await page.getByRole('button', { name: 'Show unread only' }).click()
  await expect(page.locator('[role=option]').first()).toBeVisible()

  const rowsBefore = await page.locator('[role=option]').count()
  const first = page.locator('[role=option]').first()
  // By id: the row's label begins 'Unread.' and changes the moment the message is read.
  const id = await first.getAttribute('data-message-id')

  await first.click()

  // Long enough for the old loop to have read four or five messages.
  await page.waitForTimeout(4_000)

  // The selection did not move by itself…
  await expect(page.locator(selected)).toHaveAttribute('data-message-id', id ?? '')

  // …and the list lost nothing: the open message stays, read, until the selection moves.
  await expect(page.locator('[role=option]')).toHaveCount(rowsBefore)
})

test('the message read under the filter leaves once the selection moves on', async ({ page }) => {
  await page.goto('/')
  await expect(page.locator('[role=option]').first()).toBeVisible()

  await page.getByRole('button', { name: 'Show unread only' }).click()
  await expect(page.locator('[role=option]').first()).toBeVisible()

  const firstId = await page.locator('[role=option]').first().getAttribute('data-message-id')
  await page.locator('[role=option]').first().click()
  await page.waitForTimeout(1_500)

  // Down to the next one, which is then read in its turn and kept; the first is now read and
  // not selected, so it is no longer something the filter keeps.
  await page.keyboard.press('ArrowDown')
  await page.waitForTimeout(1_500)

  await expect(page.locator(`[role=option][data-message-id="${firstId ?? ''}"]`)).toHaveCount(0)
})
