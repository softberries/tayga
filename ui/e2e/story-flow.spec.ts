import { expect, test } from './fixtures'

test('home inspector to story to trace to span drawer shows attributes', async ({ page }) => {
  await page.goto('/')
  const inspector = page.getByRole('complementary', { name: 'Selected story' })
  await inspector.getByRole('link', { name: 'Open story' }).click()

  await expect(page).toHaveURL(/\/stories\/[0-9a-f]{32}/)
  await expect(page.getByRole('heading', { level: 1 })).toHaveText(/^Story /)
  await expect(page.getByRole('treeitem').first()).toBeVisible()

  await page.getByRole('link', { name: /^Open trace/ }).click()
  await expect(page).toHaveURL(/\/traces\/[0-9a-f]{32}/)
  await expect(page.getByRole('heading', { level: 1 })).toHaveText(/^Trace /)

  const rows = page.getByRole('treeitem')
  await expect(rows.first()).toBeVisible()

  // OTLP does not require span attributes, so the root span may have none. Walk the first few
  // spans: an empty one shows the empty state; the first one with attributes shows a key/value pair.
  const drawer = page.getByRole('dialog')
  const limit = Math.min(await rows.count(), 5)
  let found = false
  for (let i = 0; i < limit && !found; i++) {
    await rows.nth(i).click()
    await expect(drawer).toBeVisible()
    const attributes = drawer.getByRole('tab', { name: /^Attributes/ })
    await expect(attributes).toHaveAttribute('aria-selected', 'true')
    const count = Number(((await attributes.textContent()) ?? '').replace(/\D/g, ''))
    if (count === 0) {
      await expect(drawer.getByRole('tabpanel')).toContainText('No attributes on this span')
      await page.keyboard.press('Escape')
      await expect(drawer).toBeHidden()
      continue
    }
    await expect(attributes).toHaveText(/Attributes\s*[1-9]/)
    await expect(drawer.getByRole('tabpanel').locator('dt, th, td, span').first()).toBeVisible()
    found = true
  }
  expect(found, `none of the first ${limit} spans has attributes`).toBe(true)

  await page.keyboard.press('Escape')
  await expect(drawer).toBeHidden()
})
