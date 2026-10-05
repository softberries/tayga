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
  await rows.first().click()

  const drawer = page.getByRole('dialog')
  await expect(drawer).toBeVisible()
  const attributes = drawer.getByRole('tab', { name: /^Attributes/ })
  await expect(attributes).toHaveAttribute('aria-selected', 'true')
  // A positive count, and a rendered key/value pair in the panel.
  await expect(attributes).toHaveText(/Attributes\s*[1-9]/)
  await expect(drawer.getByRole('tabpanel')).toContainText(/\S+/)
  await expect(drawer.getByRole('tabpanel').locator('dt, th, td, span').first()).toBeVisible()

  await page.keyboard.press('Escape')
  await expect(drawer).toBeHidden()
})
