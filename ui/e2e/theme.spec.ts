import { expect, test } from './fixtures'

test('theme switch persists across reload', async ({ page, theme }) => {
  const other = theme === 'dark' ? 'light' : 'dark'
  await page.goto('/pipeline')
  await expect(page.locator('html')).toHaveAttribute('data-theme', theme)

  // The switch cycles light, dark, system: click until the other explicit theme is on.
  const toggle = page.getByRole('button', { name: /^Theme:/ })
  for (let i = 0; i < 3 && (await toggle.getAttribute('data-mode')) !== other; i++) await toggle.click()
  await expect(toggle).toHaveAttribute('data-mode', other)
  await expect(page.locator('html')).toHaveAttribute('data-theme', other)
  expect(await page.evaluate(() => localStorage.getItem('tayga-theme'))).toBe(other)

  await page.reload()
  await expect(page.locator('html')).toHaveAttribute('data-theme', other)
  await expect(page.getByRole('button', { name: /^Theme:/ })).toHaveAttribute('data-mode', other)

  // A new page in the same tab keeps it too.
  await page.getByRole('link', { name: 'Traces' }).click()
  await expect(page.locator('html')).toHaveAttribute('data-theme', other)
})

test('the palette theme actions change the theme', async ({ page, theme }) => {
  const other = theme === 'dark' ? 'light' : 'dark'
  await page.goto('/')
  // Shortcuts are bound once the app has mounted.
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
  await page.keyboard.press('ControlOrMeta+K')
  await page.getByRole('option', { name: `Theme: ${other[0]!.toUpperCase()}${other.slice(1)}` }).click()
  await expect(page.locator('html')).toHaveAttribute('data-theme', other)
})
