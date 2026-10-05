import { expect, liveIds, test } from './fixtures'

test('Cmd/Ctrl+K with a real trace id navigates to the trace', async ({ page, request }) => {
  const { traceId } = await liveIds(request)
  await page.goto('/')
  await expect(page.locator('[data-fingerprint]').first()).toBeVisible()

  await page.keyboard.press('ControlOrMeta+K')
  const input = page.getByRole('combobox')
  await expect(input).toBeFocused()
  await input.fill(traceId)
  await expect(page.getByRole('option').first()).toHaveText(`Open trace ${traceId}`)
  await page.keyboard.press('Enter')

  await expect(page).toHaveURL(new RegExp(`/traces/${traceId}`))
  await expect(page.getByRole('heading', { level: 1 })).toHaveText(/^Trace /)
  await expect(page.getByRole('treeitem').first()).toBeVisible()
})

test('g then s/t/m/l/p jumps between pages', async ({ page }) => {
  await page.goto('/')
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
  for (const [key, path, heading] of [
    ['t', '/traces', /^Traces$/],
    ['m', '/map', /^Service map$/],
    ['l', '/logs/alerts', /^Alerts$/],
    ['p', '/pipeline', /^Pipeline$/],
    ['s', '/', /^Stories$/],
  ] as const) {
    await page.keyboard.press('g')
    await page.keyboard.press(key)
    await expect(page).toHaveURL((u) => u.pathname === path)
    await expect(page.getByRole('heading', { level: 1 })).toHaveText(heading)
  }
})

test('the palette list keeps its height while a search resolves', async ({ page }) => {
  await page.goto('/')
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
  await page.keyboard.press('ControlOrMeta+K')
  const area = page.getByTestId('palette-server')
  await page.getByRole('combobox').fill('frontend')
  await expect(area.getByRole('option').first()).toBeVisible()
  const withResults = (await area.boundingBox())!.height
  await page.getByRole('combobox').fill('zzzzqqqq')
  await expect(page.getByText(/^No matches for/)).toBeVisible()
  const empty = (await area.boundingBox())!.height
  // Both states fill at least the reserved minimum (9 rem), so the list never collapses.
  const rem = await page.evaluate(() => parseFloat(getComputedStyle(document.documentElement).fontSize))
  expect(empty).toBeGreaterThanOrEqual(9 * rem - 1)
  expect(withResults).toBeGreaterThanOrEqual(9 * rem - 1)
})

test('the palette list never shrinks below its minimum while a slow search is in flight', async ({ page }) => {
  // Hold the search response so "Searching…" stays on screen long enough to measure.
  await page.route('**/api/v1/search*', async (route) => {
    await new Promise((r) => setTimeout(r, 1500))
    await route.continue()
  })
  await page.goto('/')
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
  await page.keyboard.press('ControlOrMeta+K')
  const area = page.getByTestId('palette-server')
  const rem = await page.evaluate(() => parseFloat(getComputedStyle(document.documentElement).fontSize))
  const min = 9 * rem - 1

  await page.getByRole('combobox').fill('frontend')
  await expect(page.getByText('Searching…')).toBeVisible()
  const heights: number[] = [(await area.boundingBox())!.height]
  while (await page.getByText('Searching…').isVisible()) {
    heights.push((await area.boundingBox())!.height)
    await page.waitForTimeout(100)
  }
  await expect(area.getByRole('option').first()).toBeVisible()
  heights.push((await area.boundingBox())!.height)
  expect(heights.length).toBeGreaterThan(3)
  expect(Math.min(...heights)).toBeGreaterThanOrEqual(min)
})
