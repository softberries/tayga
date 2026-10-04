import { expect, test } from '@playwright/test'

for (const theme of ['light', 'dark'] as const) {
  test(`shell renders in ${theme} without console errors`, async ({ page }) => {
    const errors: string[] = []
    page.on('console', (m) => {
      if (m.type() === 'error') errors.push(m.text())
    })
    await page.addInitScript((t) => window.localStorage.setItem('tayga-theme', t), theme)
    await page.goto('/pipeline')
    await expect(page.locator('html')).toHaveAttribute('data-theme', theme)
    await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible()
    await expect(page.getByRole('radiogroup', { name: 'Time range' })).toBeVisible()
    expect(errors).toEqual([])
  })
}
