import { screen, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { renderApp, stubApi } from '../../test/renderApp'

// The Pipeline page throws while rendering; every other page renders its placeholder.
vi.mock('../../pages/placeholders', async (importOriginal) => {
  const real = await importOriginal<typeof import('../../pages/placeholders')>()
  return {
    ...real,
    PipelinePage: () => {
      throw new Error('pipeline page exploded')
    },
  }
})

beforeEach(() => {
  // React logs caught render errors; keep the output readable.
  vi.spyOn(console, 'error').mockImplementation(() => {})
})
afterEach(() => vi.unstubAllGlobals())

function expectShell() {
  expect(screen.getByRole('navigation', { name: 'Main' })).toBeInTheDocument()
  expect(screen.getByRole('radiogroup', { name: 'Time range' })).toBeInTheDocument()
  expect(screen.getByRole('button', { name: /^Theme:/ })).toBeInTheDocument()
}

describe('error boundaries', () => {
  it('a page that throws fails inside the shell', async () => {
    stubApi({ '/service-map': { body: { edges: [], nodes: [] } } })
    renderApp('/pipeline')
    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent('pipeline page exploded')
    expect(within(alert).getByRole('button', { name: 'Try again' })).toBeInTheDocument()
    expectShell()
    expect(screen.getByRole('main')).toContainElement(alert)
  })

  it('other pages still render after navigating away', async () => {
    stubApi({ '/service-map': { body: { edges: [], nodes: [] } } })
    const { router } = renderApp('/pipeline')
    await screen.findByRole('alert')
    await router.navigate({ to: '/map' })
    expect(await screen.findByText('No service calls in this window')).toBeInTheDocument()
  })

  it('an old /service-map shape (edges array) leaves the shell intact', async () => {
    // The pre-plan-5 API returned a bare array of edges.
    stubApi({ '/service-map': { body: [{ parent: 'a', child: 'b', calls: 1, errors: 0 }] } })
    renderApp('/logs/alerts')
    expect(await screen.findByText('This page is built in Task 10.')).toBeInTheDocument()
    expectShell()
    expect(screen.queryByText(/degraded/)).toBeNull()
  })
})
