import type { Page } from '@playwright/test'
import { expect, test } from './fixtures'

const running = (page: Page) =>
  page.evaluate(() =>
    document
      .getAnimations()
      .filter((a) => a.playState === 'running')
      .map((a) => {
        const t = (a.effect as KeyframeEffect | null)?.target as Element | null
        return `${t?.tagName.toLowerCase() ?? '?'}.${String(t?.getAttribute('class') ?? '').slice(0, 50)} ${(a as CSSAnimation).animationName ?? a.constructor.name}`
      }),
  )

test.describe('reduced motion', () => {
  test.skip(({ reducedMotion }) => reducedMotion !== 'reduce', 'runs in the reduced-motion project')

  for (const path of ['/', '/traces', '/map', '/logs/alerts', '/pipeline']) {
    test(`${path} has no running animations, while loading or settled`, async ({ page }) => {
      expect(await page.evaluate(() => matchMedia('(prefers-reduced-motion: reduce)').matches)).toBe(true)
      await page.goto(path)
      // Straight after load, while skeletons and fades would still be running.
      expect(await running(page)).toEqual([])
      await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
      await page.waitForLoadState('networkidle')
      await page.waitForTimeout(800)
      expect(await running(page)).toEqual([])
    })
  }

  test('opening the palette and a drawer starts no animation', async ({ page }) => {
    await page.goto('/map')
    await expect(page.locator('[data-service]').first()).toBeVisible()
    await page.keyboard.press('ControlOrMeta+K')
    await expect(page.getByRole('dialog')).toBeVisible()
    expect(await running(page)).toEqual([])
    await page.keyboard.press('Escape')
    await page.locator('[data-service]').first().click()
    await expect(page.getByRole('dialog')).toBeVisible()
    expect(await running(page)).toEqual([])
  })
})
