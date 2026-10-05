/**
 * Login, session and sign-out against a tayga-api with auth enabled (spec §2.4). Skipped
 * unless TAYGA_E2E_AUTH_USER and TAYGA_E2E_AUTH_PASS are set, e.g. for the Vite dev server
 * proxied to an auth-enabled API:
 *
 *   TAYGA_E2E_AUTH_USER=admin TAYGA_E2E_AUTH_PASS=… TAYGA_UI_URL=http://localhost:5173 \
 *     npx playwright test auth.spec.ts --project=dark --project=light
 *
 * Each run spends one failed login per project, and the API allows 5 per IP in 5 minutes.
 */
import { expect, test as base, trackProblems } from './fixtures'
import type { Page } from '@playwright/test'

const USER = process.env.TAYGA_E2E_AUTH_USER
const PASS = process.env.TAYGA_E2E_AUTH_PASS

/** The 401s this flow causes on purpose: the guard's session probe and the wrong password. */
const EXPECTED_401 = /\/api\/v1\/auth\/(me|login)$/

const test = base.extend({
  // Replaces the shared fixture: the same checks, minus the expected 401s and their console lines.
  problems: async ({ page }, use) => {
    const problems = trackProblems(page)
    const expected = new Set<string>()
    page.on('response', (r) => {
      if (r.status() === 401 && EXPECTED_401.test(new URL(r.url()).pathname)) expected.add(r.url())
    })
    await use(problems)
    const unexpected = problems.filter((p) => {
      if (p.startsWith('HTTP 401: ')) return !expected.has(p.slice('HTTP 401: '.length))
      // Chrome logs every failed fetch as "Failed to load resource: … status of 401".
      if (p.startsWith('console: ') && p.includes('status of 401')) return expected.size === 0
      return true
    })
    expect(unexpected, 'console errors and failed requests').toEqual([])
  },
})

test.skip(!USER || !PASS, 'set TAYGA_E2E_AUTH_USER and TAYGA_E2E_AUTH_PASS to run against an auth-enabled API')
// Requesting `problems` here keeps the overridden checks on for every test.
test.beforeEach(({ problems }) => {
  void problems
  const project = test.info().project.name
  test.skip(project !== 'dark' && project !== 'light', 'runs in the dark and light projects')
})

const NEXT = '/map?since=24h'

async function fill(page: Page, user: string, pass: string) {
  await page.getByLabel('Username').fill(user)
  await page.getByLabel('Password').fill(pass)
  await page.getByRole('button', { name: 'Sign in' }).click()
}

test('an app page without a session redirects to login with next', async ({ page, theme }) => {
  await page.goto(NEXT)
  await expect(page).toHaveURL(`/login?next=${encodeURIComponent(NEXT)}`)
  await expect(page.getByRole('heading', { name: 'Tayga' })).toBeVisible()
  await expect(page.getByRole('navigation', { name: 'Main' })).toHaveCount(0)
  await expect(page.locator('html')).toHaveAttribute('data-theme', theme)
  await expect(page.getByLabel('Username')).toBeFocused()
  await page.waitForTimeout(500)
  await page.screenshot({ path: `e2e/screenshots/${theme}/login.png`, animations: 'disabled' })
  await page.setViewportSize({ width: 390, height: 844 })
  await page.screenshot({ path: `e2e/screenshots/${theme}/login-narrow.png`, animations: 'disabled' })
})

test('a wrong password shows the alert and clears the password', async ({ page, theme }) => {
  await page.goto('/login')
  await fill(page, USER!, `${PASS!}-wrong`)
  await expect(page.getByRole('alert')).toHaveText('Wrong username or password.')
  await expect(page.getByLabel('Username')).toHaveValue(USER!)
  await expect(page.getByLabel('Password')).toHaveValue('')
  await expect(page.getByLabel('Password')).toBeFocused()
  await expect(page).toHaveURL('/login')
  await page.waitForTimeout(500)
  await page.screenshot({ path: `e2e/screenshots/${theme}/login-error.png`, animations: 'disabled' })
})

test('the right password lands on next, a reload stays signed in, sign out returns to login', async ({ page }) => {
  await page.goto(NEXT)
  await expect(page).toHaveURL(/\/login\?next=/)
  await fill(page, USER!, PASS!)
  await expect(page).toHaveURL(NEXT)
  await expect(page.getByRole('heading', { level: 1, name: 'Service map' })).toBeVisible()
  const chip = page.getByRole('button', { name: `Signed in as ${USER!}` })
  await expect(chip).toBeVisible()

  await page.reload()
  await expect(page).toHaveURL(NEXT)
  await expect(chip).toBeVisible()

  await chip.click()
  await page.getByRole('menuitem', { name: 'Sign out' }).click()
  await expect(page).toHaveURL('/login')
  await expect(page.getByRole('heading', { name: 'Tayga' })).toBeVisible()

  // The session is gone on the server too.
  await page.goto('/pipeline')
  await expect(page).toHaveURL(`/login?next=${encodeURIComponent('/pipeline')}`)
})

test('an unsafe next lands on /', async ({ page }) => {
  await page.goto(`/login?next=${encodeURIComponent('//evil.example')}`)
  await fill(page, USER!, PASS!)
  await expect(page).toHaveURL('/')
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
})
