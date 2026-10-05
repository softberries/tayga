import { defineConfig, devices } from '@playwright/test'
import type { Theme } from './e2e/fixtures'

// Runs against a deployed stack: tayga-api serving the embedded app on :8090 (`make up`).
// Override the target with TAYGA_UI_URL, e.g. TAYGA_UI_URL=http://localhost:5173 for the Vite
// dev server (started with TAYGA_API=http://localhost:8090 so /api is proxied to the stack).
//
// Projects: `perf` measures the budgets alone (the others wait for it, so nothing else loads the
// machine meanwhile); `dark` and `light` run every spec with the theme stored in localStorage
// before load; `reduced-motion` runs them again with prefers-reduced-motion emulated.
const baseURL = process.env.TAYGA_UI_URL ?? 'http://localhost:8090'

const desktop = (theme: Theme, reducedMotion: 'no-preference' | 'reduce' = 'no-preference') => ({
  ...devices['Desktop Chrome'],
  viewport: { width: 1440, height: 900 },
  reducedMotion,
  theme,
})

export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  retries: 0,
  timeout: 45_000,
  expect: { timeout: 15_000 },
  reporter: [['list']],
  outputDir: 'test-results',
  use: { baseURL, trace: 'retain-on-failure' },
  projects: [
    { name: 'perf', testMatch: /perf\.spec\.ts/, fullyParallel: false, use: desktop('dark') },
    { name: 'dark', testIgnore: /perf\.spec\.ts/, dependencies: ['perf'], use: desktop('dark') },
    { name: 'light', testIgnore: /perf\.spec\.ts/, dependencies: ['perf'], use: desktop('light') },
    { name: 'reduced-motion', testIgnore: /perf\.spec\.ts/, dependencies: ['perf'], use: desktop('dark', 'reduce') },
  ],
})
