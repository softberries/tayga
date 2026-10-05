import type { Page } from '@playwright/test'
import { expect, liveIds, test } from './fixtures'
import type { Ids } from './fixtures'

interface Route {
  name: string
  path: (ids: Ids) => string
  heading: RegExp
  /** Waits for real data on the page. */
  data: (page: Page) => Promise<void>
}

const ROUTES: Route[] = [
  {
    name: 'stories',
    path: () => '/',
    heading: /^Stories$/,
    data: async (page) => {
      await expect(page.locator('[data-fingerprint]').first()).toBeVisible()
      await expect(page.getByRole('list', { name: 'Summary' }).getByRole('listitem')).toHaveCount(4)
      await expect(page.getByRole('complementary', { name: 'Selected story' })).toBeVisible()
    },
  },
  {
    name: 'story',
    path: (ids) => `/stories/${ids.storyId}`,
    heading: /^Story /,
    data: async (page) => {
      await expect(page.getByRole('treeitem').first()).toBeVisible()
      await expect(page.getByRole('heading', { level: 2 }).first()).toBeVisible()
    },
  },
  {
    name: 'traces',
    path: () => '/traces',
    heading: /^Traces$/,
    data: async (page) => {
      await expect(page.locator('canvas').first()).toBeVisible()
      await expect(page.getByRole('table').getByRole('row').nth(2)).toBeVisible()
    },
  },
  {
    name: 'trace',
    path: (ids) => `/traces/${ids.traceId}`,
    heading: /^Trace /,
    data: async (page) => {
      await expect(page.getByRole('treeitem').first()).toBeVisible()
    },
  },
  {
    name: 'map',
    path: () => '/map',
    heading: /^Service map$/,
    data: async (page) => {
      await expect(page.locator('[data-service]').first()).toBeVisible()
    },
  },
  {
    name: 'log-alerts',
    // A day, not the default hour: alerts age out of the last hour between scenario runs.
    path: () => '/logs/alerts?since=24h',
    heading: /^Alerts$/,
    data: async (page) => {
      await expect(page.getByRole('table', { name: 'Log alerts' }).getByRole('row').nth(1)).toBeVisible()
    },
  },
  {
    name: 'log-templates',
    path: () => '/logs/templates',
    heading: /^Templates$/,
    data: async (page) => {
      await expect(page.getByRole('table', { name: 'Templates' }).getByRole('row').nth(1)).toBeVisible()
    },
  },
  {
    name: 'log-template',
    path: (ids) => `/logs/templates/${ids.templateId}`,
    heading: /^Template /,
    data: async (page) => {
      await expect(page.locator('canvas').first()).toBeVisible()
    },
  },
  {
    name: 'pipeline',
    path: () => '/pipeline',
    heading: /^Pipeline$/,
    data: async (page) => {
      await expect(page.getByRole('list', { name: 'Job status' }).getByRole('listitem')).toHaveCount(5)
      await expect(page.locator('canvas').first()).toBeVisible()
    },
  },
]

let ids: Ids
test.beforeAll(async ({ request }) => {
  ids = await liveIds(request)
})

for (const route of ROUTES) {
  test(`${route.name}: renders its heading and data, no console errors, screenshot`, async ({ page, theme }, info) => {
    await page.goto(route.path(ids))
    await expect(page.locator('html')).toHaveAttribute('data-theme', theme)
    await expect(page.getByRole('heading', { level: 1 })).toHaveText(route.heading)
    await route.data(page)
    // Let skeleton fades and chart animations finish before the picture.
    await page.waitForLoadState('networkidle')
    await page.waitForTimeout(900)
    // Screenshots are for review (gitignored, never compared); the reduced-motion run adds none.
    if (info.project.name === 'dark' || info.project.name === 'light') {
      await page.screenshot({ path: `e2e/screenshots/${theme}/${route.name}.png`, animations: 'disabled' })
    }
  })
}
