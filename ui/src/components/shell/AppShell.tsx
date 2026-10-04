import { Outlet } from '@tanstack/react-router'
import { Header } from './Header'
import { OutageBanner } from './OutageBanner'
import { Rail } from './Rail'

export function AppShell() {
  return (
    <div className="flex min-h-dvh bg-ground text-ink max-sm:flex-col">
      <a
        href="#main"
        className="sr-only focus:not-sr-only focus:fixed focus:left-2 focus:top-2 focus:z-50 focus:rounded-control focus:bg-panel focus:px-3 focus:py-2"
      >
        Skip to content
      </a>
      <Rail />
      <div className="flex min-w-0 flex-1 flex-col">
        <Header />
        <OutageBanner />
        <main id="main" className="flex min-h-0 flex-1 flex-col px-6 py-[18px]">
          <Outlet />
        </main>
      </div>
    </div>
  )
}
