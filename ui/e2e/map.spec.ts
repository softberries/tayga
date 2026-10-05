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

// Payment and shipping never serve a trace's root (the endpoint is frontend-proxy or similar),
// so the drawer's link must match the service anywhere in the trace to find any.
for (const service of ['payment', 'shipping']) {
  test(`"Open ${service} traces" from the drawer lists traces that pass through it`, async ({ page }) => {
    await page.goto(`/map?service=${service}`)
    const drawer = page.getByRole('dialog', { name: service })
    await expect(drawer).toBeVisible()
    await drawer.getByRole('link', { name: `Open ${service} traces` }).click()

    await expect(page).toHaveURL(new RegExp(`/traces\\?service=${service}&touched=true$`))
    await expect(page.getByRole('radiogroup', { name: 'Service match' }).getByRole('radio', { name: 'Anywhere in trace' })).toHaveAttribute('data-state', 'on')
    await expect(page.getByRole('table').getByRole('row').nth(1)).toBeVisible()
    await expect(page.getByText(/^\d+ traces?( \(limit reached\))?$/)).toBeVisible()
    await expect(page.getByText('No traces match')).toBeHidden()
  })
}

test('infrastructure (flagd) is hidden by default and drawn with infra=true', async ({ page }) => {
  await page.goto('/map')
  await expect(page.locator('[data-service]').first()).toBeVisible()
  expect(await page.locator('[data-service]').count()).toBeGreaterThan(3)
  await expect(page.locator('[data-service="flagd"]')).toHaveCount(0)
  // Its callers say so.
  await expect(page.locator('[data-infra-badge]').first()).toBeVisible()

  await page.getByRole('switch', { name: 'Show infrastructure' }).click()
  await expect(page).toHaveURL(/[?&]infra=true/)
  await expect(page.locator('[data-service="flagd"]')).toBeVisible()
  await expect(page.locator('[data-infra-badge]')).toHaveCount(0)

  await page.goto('/map?infra=true')
  await expect(page.locator('[data-service="flagd"]')).toBeVisible()
  await expect(page.getByRole('switch', { name: 'Show infrastructure' })).toBeChecked()
})

test('/map?service=flagd opens flagd even while infrastructure is hidden', async ({ page }) => {
  await page.goto('/map?service=flagd')
  await expect(page.getByRole('dialog', { name: 'flagd' })).toBeVisible()
  await expect(page.getByRole('switch', { name: 'Show infrastructure' })).not.toBeChecked()
})
