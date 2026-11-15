import { expect, test } from '@playwright/test'

/**
 * The sidebar's Tab stop when the selected row is folded away.
 *
 * L27 in the 2026-11-14 audit: the one tabbable row was the selected one, so collapsing the row
 * it sat under left the tree with no Tab stop, and the sidebar could not be reached by keyboard.
 */

test('folding the selected row away leaves the row it is folded into tabbable', async ({
  page,
}) => {
  await page.goto('/')
  await expect(page.getByRole('listbox', { name: 'Messages' })).toBeVisible()

  const tree = page.getByRole('tree', { name: 'Mailboxes' })
  // Found by being open, then held by its id: an `aria-expanded="true"` locator would move on
  // to the next open row the moment this one closes.
  const open = tree.locator('[role="treeitem"][aria-expanded="true"]').first()
  const parentLevel = Number(await open.getAttribute('aria-level'))
  const parentId = (await open.getAttribute('data-node-id')) ?? ''
  const parent = tree.locator(`[data-node-id="${parentId}"]`)

  // The parent's first child: the next row, one level down.
  const child = parent.locator(
    `xpath=following-sibling::*[@role="treeitem"][@aria-level="${String(parentLevel + 1)}"][1]`,
  )
  await child.click()
  await expect(child).toHaveAttribute('aria-selected', 'true')

  await parent.getByRole('button', { name: /^Collapse / }).click()
  await expect(parent).toHaveAttribute('aria-expanded', 'false')

  const tabbable = tree.locator('[role="treeitem"][tabindex="0"]')
  await expect(tabbable).toHaveCount(1)
  await expect(tabbable).toHaveAttribute('data-node-id', parentId)
})
