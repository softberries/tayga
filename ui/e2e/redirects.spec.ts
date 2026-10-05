import { expect, test } from './fixtures'

for (const [from, to] of [
  ['/service-map', '/map'],
  ['/alerts', '/logs/alerts'],
  ['/templates', '/logs/templates'],
  ['/templates?since=24h', '/logs/templates?since=24h'],
] as const) {
  test(`${from} answers 308 to ${to}`, async ({ request, page }) => {
    const res = await request.get(from, { maxRedirects: 0 })
    expect(res.status()).toBe(308)
    expect(res.headers()['location']).toBe(to)

    await page.goto(from)
    await expect(page).toHaveURL((u) => `${u.pathname}${u.search}` === to)
    await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
  })
}
