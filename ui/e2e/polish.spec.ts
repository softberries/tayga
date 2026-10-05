import { expect, test } from './fixtures'

test('phone bottom bar tooltips open upward', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 })
  await page.goto('/')
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
  await page.getByRole('navigation', { name: 'Main' }).getByRole('link', { name: 'Traces' }).focus()
  const tip = page.getByRole('tooltip')
  await expect(tip).toHaveText('Traces')
  await expect(tip).toHaveAttribute('data-side', 'top')
})

test('the desktop rail tooltip opens to the right', async ({ page }) => {
  await page.goto('/')
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
  await page.getByRole('navigation', { name: 'Main' }).getByRole('link', { name: 'Traces' }).focus()
  await expect(page.getByRole('tooltip')).toHaveAttribute('data-side', 'right')
})

test('a truncated template shows its full text in a tooltip, a short one shows none', async ({ page }) => {
  await page.goto('/logs/templates')
  const links = page.getByRole('table', { name: 'Templates' }).getByRole('link')
  await expect(links.first()).toBeVisible()
  const cut = await links.evaluateAll((els) => els.map((e) => e.scrollWidth > e.clientWidth))
  expect(cut.some(Boolean), 'the live templates include a truncated one').toBe(true)

  const truncated = links.nth(cut.indexOf(true))
  const full = (await truncated.textContent())!
  await truncated.hover()
  await expect(page.getByRole('tooltip')).toHaveText(full)

  if (cut.includes(false)) {
    // Radix closes a hoverable tooltip on the first pointer move after leaving, so move in steps.
    await page.mouse.move(5, 5, { steps: 5 })
    await expect(page.getByRole('tooltip')).toHaveCount(0)
    await links.nth(cut.indexOf(false)).hover()
    await page.waitForTimeout(900)
    await expect(page.getByRole('tooltip')).toHaveCount(0)
  }
})

test('arrowing to a truncated story row exposes the full text', async ({ page }) => {
  await page.goto('/')
  const rows = page.locator('[data-fingerprint]')
  await expect(rows.first()).toBeVisible()
  // The summary of each row as the DOM holds it (CSS truncation does not shorten it).
  const cut = await rows.evaluateAll((els) =>
    els.map((r) => ({ id: r.getAttribute('data-fingerprint')!, truncated: [...r.querySelectorAll('.truncate')].some((e) => e.scrollWidth > e.clientWidth) })),
  )
  // A truncated row below the first, so the arrow keys have to move focus to it.
  const index = cut.findIndex((c, i) => i > 0 && c.truncated)
  expect(index, 'the live stories include a truncated row below the first').toBeGreaterThan(0)
  const row = rows.nth(index)

  // Keyboard only: Tab to the grid's one tab stop (the selected row), then ArrowDown to the
  // target. Rows use a roving tabindex.
  const first = rows.first()
  for (let i = 0; i < 80; i++) {
    if (await first.evaluate((e) => e === document.activeElement)) break
    await page.keyboard.press('Tab')
  }
  await expect(first).toBeFocused()
  for (let i = 0; i < index; i++) await page.keyboard.press('ArrowDown')
  await expect(row).toBeFocused()
  const tip = page.getByRole('tooltip').first()
  await expect(tip).toBeVisible()
  const text = (await tip.textContent())!
  expect(text.length).toBeGreaterThan(10)
  // What the tooltip shows is part of the row, whole.
  expect((await row.textContent())!.replace(/\s+/g, ' ')).toContain(text.replace(/\s+/g, ' ').trim().slice(0, 30))
})
