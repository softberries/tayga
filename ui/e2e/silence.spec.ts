/**
 * Silence alerts in the UI: the template page switch, the silence kind in the alert list and
 * its filter, and the bell in the templates table. The stack under test may be an API build
 * from before the silence route, so the silence parts of its answers are layered on top of the
 * live responses here (the live data stays the base).
 */
import { expect, test } from './fixtures'
import type { Page } from '@playwright/test'
import { liveIds } from './fixtures'

const SHOTS = process.env.TAYGA_SHOTS_DIR ?? 'e2e/screenshots'

interface Setting {
  enabled: boolean
  minutes: number
}

/** A fake silence store over the live API: PUT updates it, GETs of the template show it. */
async function mockSilence(page: Page, templateId: string) {
  const state: { setting: Setting | null; puts: Array<{ body: unknown; contentType: string | undefined }> } = { setting: null, puts: [] }
  const now = Date.now() * 1e6
  const silenceAlert = (template: string, service: string) => ({
    alert_id: 'e2e-silence-1',
    kind: 'silence',
    template_id: templateId,
    service,
    template,
    started_at_ns: now - 25 * 60e9,
    last_at_ns: now - 5 * 60e9,
    window_count: 0,
    peak_count: 0,
    baseline_per_window: 0,
    active: true,
    example_traces: [],
  })

  await page.route(new RegExp(`/api/v1/log-templates/${templateId}/silence$`), async (route) => {
    const req = route.request()
    const body = req.postDataJSON() as Setting
    state.puts.push({ body, contentType: req.headers()['content-type'] })
    state.setting = body
    await route.fulfill({ json: body })
  })
  await page.route(new RegExp(`/api/v1/log-templates/${templateId}\\?`), async (route) => {
    const res = await route.fetch()
    const json = (await res.json()) as Record<string, unknown>
    await route.fulfill({ response: res, json: { ...json, silence: state.setting } })
  })
  await page.route(/\/api\/v1\/log-templates\?/, async (route) => {
    const res = await route.fetch()
    const rows = (await res.json()) as Array<{ template_id: string }>
    await route.fulfill({
      response: res,
      json: rows.map((r) => ({ ...r, silence_enabled: r.template_id === templateId && state.setting?.enabled === true })),
    })
  })
  await page.route(/\/api\/v1\/log-alerts\?/, async (route) => {
    const detail = await (await page.request.get(`/api/v1/log-templates/${templateId}?since=1h`)).json()
    const extra = [silenceAlert(detail.template.template, detail.template.service)]
    const kind = new URL(route.request().url()).searchParams.get('kind')
    // An API from before the silence kind answers 400 to kind=silence: never forward it.
    if (kind === 'silence') return route.fulfill({ json: extra })
    const res = await route.fetch()
    const live = (await res.json()) as Array<Record<string, unknown>>
    await route.fulfill({ response: res, json: kind ? live : [...extra, ...live] })
  })
  return state
}

test('the template page switch saves the setting with one JSON PUT and survives a reload', async ({ page, request, theme }) => {
  const { templateId } = await liveIds(request)
  const state = await mockSilence(page, templateId)
  await page.goto(`/logs/templates/${templateId}`)
  const sw = page.getByRole('switch', { name: 'Alert when silent' })
  await expect(sw).not.toBeChecked()
  const minutes = page.getByRole('textbox', { name: 'Silent minutes' })
  await expect(minutes).toBeDisabled()

  await sw.click()
  await minutes.fill('0')
  await expect(page.getByRole('alert')).toContainText('from 1 to 1440')
  await expect(page.getByRole('button', { name: 'Save' })).toBeDisabled()
  await minutes.fill('15')
  await page.getByRole('button', { name: 'Save' }).click()
  await expect(page.getByRole('status')).toContainText('Saved: alerts after 15 min of silence')
  expect(state.puts).toEqual([{ body: { enabled: true, minutes: 15 }, contentType: 'application/json' }])

  await page.reload()
  await expect(page.getByRole('switch', { name: 'Alert when silent' })).toBeChecked()
  await expect(page.getByRole('textbox', { name: 'Silent minutes' })).toHaveValue('15')
  await page.waitForLoadState('networkidle')
  await page.waitForTimeout(900)
  await page.screenshot({ path: `${SHOTS}/t7b-silence-${theme}.png`, animations: 'disabled' })
})

test('the alert list shows silence alerts, filters by kind in the URL, and the templates table has a bell', async ({ page, request, theme }) => {
  const { templateId } = await liveIds(request)
  const state = await mockSilence(page, templateId)
  state.setting = { enabled: true, minutes: 10 }

  await page.goto('/logs/alerts?since=24h')
  const table = page.getByRole('table', { name: 'Log alerts' })
  const silent = table.getByRole('row').filter({ hasText: 'silent 20 min' })
  await expect(silent).toHaveCount(1)
  await expect(silent.getByText('silence', { exact: true })).toHaveAttribute('data-kind', 'silence')
  await page.waitForLoadState('networkidle')
  await page.waitForTimeout(900)
  await page.screenshot({ path: `${SHOTS}/t7b-alerts-silence-${theme}.png`, animations: 'disabled' })

  await page.getByRole('radio', { name: 'Silence' }).click()
  await expect(page).toHaveURL(/kind=silence/)
  await expect(table.getByRole('row')).toHaveCount(2)
  await page.reload()
  await expect(page.getByRole('radio', { name: 'Silence' })).toBeChecked()
  await expect(table.getByRole('row').filter({ hasText: 'silent 20 min' })).toHaveCount(1)

  await page.goto('/logs/templates')
  const row = page.locator(`[data-template-id="${templateId}"]`)
  await expect(row.getByText('alerts when silent')).toHaveCount(1)
  await expect(page.getByText('alerts when silent')).toHaveCount(1)
})
