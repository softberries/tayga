import { Breadcrumb } from './Breadcrumb'
import { CommandTrigger } from './CommandTrigger'
import { DegradedBadge } from './DegradedBadge'
import { LiveToggle } from './LiveToggle'
import { ThemeSwitch } from './ThemeSwitch'
import { TimeRange, useSince } from './TimeRange'
import { WidgetBoundary } from './WidgetBoundary'

export function Header() {
  const since = useSince()
  return (
    <header className="sticky top-0 z-30 flex flex-wrap items-center gap-3 border-b border-line bg-header px-6 py-3.5 backdrop-blur-[8px]">
      <Breadcrumb />
      <WidgetBoundary name="degraded-services" key={since}>
        <DegradedBadge />
      </WidgetBoundary>
      <div className="flex-[1_1_40px]" />
      <CommandTrigger />
      <TimeRange />
      <LiveToggle />
      <ThemeSwitch />
    </header>
  )
}
