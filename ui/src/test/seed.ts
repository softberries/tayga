import type { ClientConfig } from '../api/types'

export const AUTH_OFF: ClientConfig = { jaeger_url: null, grafana_url: null, auth_enabled: false, infra_services: ['flagd'] }

/**
 * The `/config` the next renderApp seeds into its query client, so the session guard resolves
 * at once instead of adding a request before every lazy page (which made loading-time
 * assertions race). stubApi takes it from its `/config` route; the default is auth off, and
 * setup.ts resets it after each test.
 */
let current: ClientConfig = AUTH_OFF

export function seed(config: ClientConfig): void {
  current = config
}

export function seededConfig(): ClientConfig {
  return current
}
