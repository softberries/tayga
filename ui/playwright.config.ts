import { defineConfig, devices } from '@playwright/test'

// Runs against a deployed stack (tayga-api serving the embedded app). Override the target with
// TAYGA_UI_URL, e.g. TAYGA_UI_URL=http://localhost:5173 for the dev server.
export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  retries: 0,
  reporter: [['list']],
  use: {
    baseURL: process.env.TAYGA_UI_URL ?? 'http://127.0.0.1:8090',
    trace: 'retain-on-failure',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
})
