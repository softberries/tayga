import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vitest/config'

// Dev proxy target: the tayga-api serving /api and /metrics. Override with TAYGA_API,
// e.g. TAYGA_API=http://127.0.0.1:18090 npm run dev.
const api = process.env.TAYGA_API ?? 'http://127.0.0.1:8090'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    proxy: {
      '/api': { target: api, changeOrigin: true },
      '/metrics': { target: api, changeOrigin: true },
    },
  },
  build: {
    outDir: 'dist',
    assetsDir: 'assets',
    emptyOutDir: true,
    // The real budget is 350 KB gzip of initial JS (spec §10); this only quiets the
    // minified-size warning for the single app chunk.
    chunkSizeWarningLimit: 800,
  },
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    include: ['src/**/*.test.{ts,tsx}'],
    css: false,
    restoreMocks: true,
    // above asyncUtilTimeout (5 s, src/test/setup.ts), so a slow query still fails inside its test
    testTimeout: 15_000,
  },
})
