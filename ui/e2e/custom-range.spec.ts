import { expect, test } from './fixtures'

/** A unix ms moment as a `datetime-local` value in the browser's (and this host's) local time. */
const local = (ms: number) => {
  const d = new Date(ms)
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`
}

test('a custom past window on the explorer: until in the URL and the API, rows, Live off', async ({ page }) => {
  await page.goto('/traces')
  await expect(page.getByRole('table').getByRole('row').nth(2)).toBeVisible()

  // Two hours ending an hour ago, on whole minutes.
  const end = Math.floor(Date.now() / 60_000) * 60_000 - 3_600_000
  const until = new Date(end).toISOString().replace(/\.\d{3}Z$/, 'Z')
  await page.getByRole('button', { name: 'Custom time range' }).click()
  const form = page.getByRole('form', { name: 'Custom time range' })
  await expect(form).toBeVisible()

  // The API's rules are checked before anything is sent.
  await form.getByLabel('From').fill(local(end))
  await form.getByLabel('To').fill(local(end - 7_200_000))
  await form.getByRole('button', { name: 'Apply' }).click()
  await expect(form.getByRole('alert')).toHaveText('The end must be after the start.')

  await form.getByLabel('From').fill(local(end - 7_200_000))
  await form.getByLabel('To').fill(local(end))
  const search = page.waitForRequest((r) => r.url().includes('/api/v1/traces/search?') && r.url().includes(`until=${encodeURIComponent(until)}`))
  await form.getByRole('button', { name: 'Apply' }).click()
  await expect(form).toBeHidden()

  await expect(page).toHaveURL(new RegExp(`[?&]until=${until.replace(/:/g, '%3A')}`))
  await expect(page).toHaveURL(/[?&]since=2h/)
  expect((await search).url()).toContain('since=2h')
  await expect(page.getByRole('button', { name: /^Custom time range: / })).toBeVisible()
  await expect(page.getByRole('table').getByRole('row').nth(2)).toBeVisible()
  await expect(page.getByText(/^\d+ traces/)).toBeVisible()

  const live = page.getByRole('button', { name: 'Live' })
  await expect(live).toHaveAttribute('aria-disabled', 'true')
  await expect(live).toHaveAttribute('aria-pressed', 'false')
  await live.hover()
  await expect(page.getByRole('tooltip')).toHaveText('Live is off for a past range')

  // Links keep the range: the rail's map link carries until.
  await page.getByRole('navigation', { name: 'Main' }).getByRole('link', { name: 'Service map' }).click()
  await expect(page).toHaveURL(/\/map\?.*until=/)

  // A preset ends now again: until is gone and Live is back.
  await page.getByRole('radiogroup', { name: 'Time range' }).getByRole('radio', { name: '15m' }).click()
  await expect(page).not.toHaveURL(/until=/)
  await expect(live).not.toHaveAttribute('aria-disabled', 'true')
})
