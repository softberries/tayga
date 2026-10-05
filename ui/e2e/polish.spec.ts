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
