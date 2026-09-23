import { expect, test, type Page } from '@playwright/test'

/**
 * Every native dropdown is readable in the dark theme.
 *
 * Reported from the compose window on 2026-09-17: opening the From picker showed a white list in
 * which every account was invisible except the one under the pointer. Windows draws an open
 * `<select>` itself — its surface from the control's own background, each row from its `<option>`
 * — and that picker was a hand-built select with `background: none`, so the list had no ground
 * and its light text landed on white. Seven other hand-built selects had the same fault; all now
 * go through `ui/Select`, and eslint refuses a new raw one.
 *
 * A screenshot cannot see the open list — it is not part of the page — so this asserts what
 * Windows paints it *from*: an opaque surface on every select, and on every option an opaque
 * background with text that can be read against it. The compose window itself cannot be
 * opened in a browser (`composeBlank` refuses outside Tauri), so its picker is covered by
 * `tests/unit/composeFrom.test.tsx`; these panes exercise the same component.
 */

type Rgba = [number, number, number, number]

function parse(colour: string): Rgba {
  const parts = /rgba?\(([^)]+)\)/
    .exec(colour)?.[1]
    ?.split(',')
    .map((part) => Number(part.trim()))
  if (parts === undefined || parts.length < 3) throw new Error(`unparseable colour: ${colour}`)
  const [r = 0, g = 0, b = 0, a = 1] = parts
  return [r, g, b, a]
}

function luminance([r, g, b]: Rgba): number {
  const channel = (value: number) => {
    const c = value / 255
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
  }
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

function contrast(a: Rgba, b: Rgba): number {
  const [light, dark] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number]
  return (light + 0.05) / (dark + 0.05)
}

async function inDarkTheme(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem(
      'halcyon.settings.display',
      JSON.stringify({
        state: { theme: 'dark', density: 'default', transparency: 'system' },
        version: 0,
      }),
    )
  })
}

/** Every select on the page, with the colours Windows will draw its open list from. */
async function paintOf(page: Page) {
  return page.evaluate(() =>
    Array.from(document.querySelectorAll('select')).map((select) => ({
      name: select.getAttribute('aria-label') ?? select.id,
      surface: getComputedStyle(select).backgroundColor,
      options: Array.from(select.options).map((option) => ({
        label: option.label,
        text: getComputedStyle(option).color,
        ground: getComputedStyle(option).backgroundColor,
      })),
    })),
  )
}

async function expectReadable(page: Page) {
  const selects = await paintOf(page)
  expect(selects.length, 'the pane should contain popups to check').toBeGreaterThan(0)

  for (const select of selects) {
    expect(parse(select.surface)[3], `${select.name}: the list needs an opaque surface`).toBe(1)

    for (const option of select.options) {
      const ground = parse(option.ground)
      expect(ground[3], `${select.name} › ${option.label}: an option needs its own ground`).toBe(1)
      expect(
        contrast(parse(option.text), ground),
        `${select.name} › ${option.label}: text must be readable on its row`,
      ).toBeGreaterThanOrEqual(4.5)
    }
  }
}

test.describe('dropdowns in the dark theme', () => {
  test('the rule editor’s popups can be read when open', async ({ page }) => {
    await inDarkTheme(page)
    await page.goto('/?settings=1&pane=rules')
    await page.getByRole('button', { name: 'Edit Rules…' }).click()
    await page.getByRole('button', { name: 'Add Rule' }).click()
    await expect(page.getByRole('combobox', { name: 'Field' })).toBeVisible()

    await expectReadable(page)
  })

  test('the account assistant’s encryption popups can be read when open', async ({ page }) => {
    await inDarkTheme(page)
    await page.goto('/?settings=1&pane=accounts')
    await page.getByRole('button', { name: 'Add Account' }).click()
    await page.getByRole('radio', { name: /Other Mail Account/ }).click()
    await page.getByRole('button', { name: 'Continue' }).click()
    await page.getByLabel('Email Address').fill('me@example.test')
    await page.getByLabel('Password').fill('secret')
    await page.getByRole('button', { name: 'Continue' }).click()
    await expect(page.getByRole('combobox', { name: 'Encryption' }).first()).toBeVisible()

    await expectReadable(page)
  })

  test('the general pane’s popups can be read when open', async ({ page }) => {
    await inDarkTheme(page)
    await page.goto('/?settings=1&pane=general')
    await expect(page.getByRole('heading', { name: 'Appearance' })).toBeVisible()

    await expectReadable(page)
  })
})

test('a rule condition’s value field has room to type in', async ({ page }) => {
  // It was laid out 0px wide. Grid sizing grew the two popup columns to their 10rem limit
  // before the flexible value column was given anything, and the 420px sheet had nothing left.
  await page.goto('/?settings=1&pane=rules')
  await page.getByRole('button', { name: 'Edit Rules…' }).click()
  await page.getByRole('button', { name: 'Add Rule' }).click()

  const value = page.getByRole('textbox', { name: 'Value' })
  await expect(value).toBeVisible()

  const width = (await value.boundingBox())?.width ?? 0
  expect(width, 'the value field should be wide enough to type a search term').toBeGreaterThan(100)

  await value.fill('invoice')
  await expect(value).toHaveValue('invoice')
})

test('an action without a second popup keeps its buttons in their own columns', async ({
  page,
}) => {
  // The × and + of such an action landed one column early — in the value column and in ×'s —
  // because the row rendered four cells in a five-column grid.
  await page.goto('/?settings=1&pane=rules')
  await page.getByRole('button', { name: 'Edit Rules…' }).click()
  await page.getByRole('button', { name: 'Add Rule' }).click()

  const conditionRemove = await page.getByRole('button', { name: 'Remove condition' }).boundingBox()
  const actionRemove = await page.getByRole('button', { name: 'Remove action' }).boundingBox()

  expect(conditionRemove).not.toBeNull()
  expect(actionRemove).not.toBeNull()
  expect(Math.abs((conditionRemove?.x ?? 0) - (actionRemove?.x ?? 0))).toBeLessThan(2)
})
