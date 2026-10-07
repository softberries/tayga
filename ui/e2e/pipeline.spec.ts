import { expect, test } from './fixtures'

test('the status strip shows 6 jobs up (ingest, writer, assembler, logminer, notifier, api) and every chart has a series', async ({ page }) => {
  await page.goto('/pipeline')
  const jobs = page.getByRole('list', { name: 'Job status' }).getByRole('listitem')
  await expect(jobs).toHaveCount(6)
  for (let i = 0; i < 6; i++) await expect(jobs.nth(i)).toHaveAttribute('data-state', 'up')

  // Every chart's text summary names a latest value, which exists only for a non-empty series.
  const captions = page.locator('figcaption')
  await expect(captions).toHaveCount(12)
  for (const text of await captions.allTextContents()) expect(text).toMatch(/ Latest: .+\d/)
  await expect(page.getByText(/^Collecting/)).toHaveCount(0)

  // Each chart painted its canvas.
  const canvases = page.locator('figure canvas')
  await expect(canvases).toHaveCount(12)
  for (let i = 0; i < 9; i++) expect((await canvases.nth(i).boundingBox())!.height).toBeGreaterThan(100)

  await expect(page.getByRole('list', { name: 'Consumer lag' })).toBeVisible()
})
