import { expect, test } from './fixtures'

test('the explorer brush filters the table rows', async ({ page }) => {
  await page.goto('/traces')
  const chart = page.locator('canvas').first()
  await expect(chart).toBeVisible()
  await expect(page.getByRole('table').getByRole('row').nth(2)).toBeVisible()
  await expect(page.getByText(/^\d+ traces/)).toBeVisible()

  const box = (await chart.boundingBox())!
  await page.mouse.move(box.x + box.width * 0.3, box.y + box.height * 0.2)
  await page.mouse.down()
  await page.mouse.move(box.x + box.width * 0.6, box.y + box.height * 0.8, { steps: 8 })
  await page.mouse.up()

  // "N of M selected" with N < M: the table now lists only the brushed traces.
  const summary = page.getByText(/^\d+ of \d+ selected/)
  await expect(summary).toBeVisible()
  const [, picked, total] = /^(\d+) of (\d+)/.exec((await summary.textContent())!)!
  expect(Number(picked)).toBeGreaterThan(0)
  expect(Number(picked)).toBeLessThan(Number(total))
  await expect(page).toHaveURL(/[?&]sel=/)

  await page.getByRole('button', { name: 'Clear selection' }).click()
  await expect(page.getByText(/^\d+ of \d+ selected/)).toBeHidden()
})
