import { expect, test } from './fixtures'

test('clicking a node opens the drawer with RED charts', async ({ page }) => {
  await page.goto('/map')
  const nodes = page.locator('[data-service]')
  await expect(nodes.first()).toBeVisible()
  expect(await nodes.count()).toBeGreaterThan(3)

  const service = (await nodes.first().getAttribute('data-service'))!
  await nodes.first().click()

  const drawer = page.getByRole('dialog')
  await expect(drawer).toBeVisible()
  await expect(drawer.getByText(service, { exact: true }).first()).toBeVisible()
  await expect(page).toHaveURL(new RegExp(`[?&]service=${encodeURIComponent(service)}`))

  // Rate, errors and p99, each a rendered canvas with a text summary.
  await expect(drawer.getByText('RED · last', { exact: false })).toBeVisible()
  const canvases = drawer.locator('canvas')
  await expect(canvases).toHaveCount(3)
  for (let i = 0; i < 3; i++) {
    const box = (await canvases.nth(i).boundingBox())!
    expect(box.width).toBeGreaterThan(100)
    expect(box.height).toBeGreaterThan(30)
  }
  await expect(drawer.locator('figcaption')).toHaveCount(3)

  await page.keyboard.press('Escape')
  await expect(drawer).toBeHidden()
})
